//! Default Reborn runtime-loop composition.

use std::{collections::HashMap, error::Error, fmt, sync::Arc};

use ironclaw_event_log::SecurityAuditSink;
use ironclaw_host_api::ids::CapabilityId;
use ironclaw_loop_contracts::{
    AgentLoopDriverError, AgentLoopHostError, CapabilitySurfaceProfileId,
    CommunicationContextProvider, InstructionSafetyContext, LoopCapabilityPort,
    LoopHostMilestoneSink, LoopModelBudgetAccountant, LoopModelPolicyGuard, LoopRunContext,
    MemoryPromptContextService, RunProfileResolver,
};
use ironclaw_loop_host::{
    AgentTurnRunCancellationFactory, AwaitEdgeSettler, AwaitEdgeWriter,
    CapabilitySurfaceProfileResolver, CompositeTurnRunWakeNotifier, HostIdentityContextSource,
    HostInputQueue, HostInputQueueReconcile, HostManagedModelGateway,
    HostManagedPromptDiagnosticSink, HostSkillContextSource, HostUserProfileSource,
    LoopAttachmentReadPort, LoopCapabilityPortDecorator, LoopCapabilityPortFactory,
    LoopCapabilityResultWriter, ModelRouteResolver, ProductLiveCancellationReadiness,
    RunCancellationFactory, SpawnSubagentFlavorDescriptor, SpawnSubagentInputCodec,
    SubagentDefinitionResolver, SubagentPromptComposer, SubagentPromptMaterialSource,
    SubagentSpawnCapabilityPort, SubagentSpawnDeps, SubagentSpawnLimits,
    ToolDisclosureCapabilityDecorator, ToolDisclosureMode, verify_product_live_cancellation_probe,
};
use ironclaw_memory::MemoryService;
use ironclaw_outbound::ReplyAttachmentIntentPort;
use ironclaw_processes::ProcessTransitionPort;
use ironclaw_threads::{SessionThreadService, ThreadScope};
use ironclaw_turns::{
    AgentTurnProcessCommitObserver, AgentTurnRuntimePort, AgentTurnSpawnTreeRuntimePort,
    DefaultTurnCoordinator, LoopCheckpointStore, TurnCommittedEventObserver, TurnEventSink,
    TurnRunWakeNotifier, TurnSpawnTreePort,
    loop_exit::{LoopExitApplier, LoopExitEvidencePort},
};

use crate::{
    app_loop_family::build_loop_family_registry_with_overrides,
    driver_registry::{DriverRegistry, DriverRegistryError},
    loop_driver_host::{
        HookDispatcherBuilderFactory, RebornLoopDriverHostFactory, TextOnlyLoopHostConfig,
        apply_capability_surface_policy, capability_resolve_error_to_agent_host_error,
    },
    loop_exit_applier::{AwaitDependentRunEvidenceStore, ThreadCheckpointLoopExitEvidencePort},
    planned_driver_factory::{
        DefaultPlannedDriverRegistrationError, default_planned_run_profile_resolver,
        register_default_planned_driver, register_default_text_only_driver,
        register_subagent_planned_driver,
    },
    subagent::{
        capability_surface::SubagentCapabilitySurfaceResolver, flavors,
        prompt_material::GateBackedSubagentPromptMaterialSource,
    },
    text_loop_driver::TextOnlyModelReplyDriverConfig,
    turn_run_executor::RebornTurnRunExecutor,
    turn_scheduler::{
        SchedulerTurnRunWakeNotifier, TurnRunScheduler, TurnRunSchedulerConfig,
        TurnRunSchedulerHandle, TurnRunWakeChannel,
    },
};

mod process_system;
pub use process_system::ProcessRuntimeSystem;

/// Default number of turn-runner worker tasks spawned per runtime instance.
///
/// Used by [`DefaultPlannedRuntimeConfig`] and [`TurnRunnerSettings`] so the
/// value is defined exactly once and shared across all crates in the stack.
pub const DEFAULT_TURN_RUNNER_WORKER_COUNT: std::num::NonZeroUsize =
    match std::num::NonZeroUsize::new(16) {
        Some(v) => v,
        // 16 is a non-zero compile-time constant so this arm is never reached.
        // `NonZeroUsize::MIN` (= 1) is used as a non-panicking fallback so the
        // CI "no panics in production code" check stays green.
        None => std::num::NonZeroUsize::MIN,
    };

/// Default per-`(tenant, ScheduledTrigger)` concurrency cap.
///
/// Set below [`DEFAULT_TURN_RUNNER_WORKER_COUNT`] so background triggers can
/// never occupy the whole scheduler pool: `worker_count - trigger_cap` slots
/// (16 - 8 = 8) stay reserved for live conversation runs.
pub const DEFAULT_MAX_CONCURRENT_TRIGGER_RUNS: std::num::NonZeroU32 =
    match std::num::NonZeroU32::new(8) {
        Some(v) => v,
        // 8 is a non-zero compile-time constant so this arm is never reached.
        None => std::num::NonZeroU32::MIN,
    };

/// Default per-`(tenant, owner user)` concurrency cap so a single user (or a
/// thread-storm) cannot monopolise the shared scheduler pool.
pub const DEFAULT_MAX_CONCURRENT_RUNS_PER_USER: std::num::NonZeroU32 =
    match std::num::NonZeroU32::new(3) {
        Some(v) => v,
        // 3 is a non-zero compile-time constant so this arm is never reached.
        None => std::num::NonZeroU32::MIN,
    };

/// Environment variable that enables bounded concurrent capability batches.
pub const REBORN_PARALLEL_TOOL_BATCH_ENV: &str = "REBORN_PARALLEL_TOOL_BATCH";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ParallelToolBatchMode {
    #[default]
    Off,
    On,
}

impl ParallelToolBatchMode {
    pub fn from_env() -> Self {
        match std::env::var(REBORN_PARALLEL_TOOL_BATCH_ENV) {
            Ok(value) => Self::from_raw(Some(&value)),
            Err(std::env::VarError::NotPresent) => Self::Off,
            Err(std::env::VarError::NotUnicode(_)) => {
                tracing::debug!(
                    target: "ironclaw::reborn::runtime",
                    env = REBORN_PARALLEL_TOOL_BATCH_ENV,
                    "REBORN_PARALLEL_TOOL_BATCH is not valid UTF-8; falling back to Off"
                );
                Self::Off
            }
        }
    }

    fn from_raw(raw: Option<&str>) -> Self {
        match raw.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            Some("on" | "1" | "true") => Self::On,
            None | Some("" | "off" | "0" | "false") => Self::Off,
            Some(_) => {
                tracing::debug!(
                    target: "ironclaw::reborn::runtime",
                    env = REBORN_PARALLEL_TOOL_BATCH_ENV,
                    "unrecognized REBORN_PARALLEL_TOOL_BATCH value; falling back to Off"
                );
                Self::Off
            }
        }
    }

    fn enables_parallel_batches(self) -> bool {
        matches!(self, Self::On)
    }
}

#[derive(Debug, Clone)]
pub struct DefaultPlannedRuntimeConfig {
    pub heartbeat_interval: std::time::Duration,
    pub poll_interval: std::time::Duration,
    /// How often the scheduler sweeps for runs whose lease has expired
    /// (`TurnRunSchedulerConfig::lease_recovery_interval`). Defaults to the
    /// scheduler's own 10s default so leaving this untouched is byte-identical
    /// to today's behavior.
    pub lease_recovery_interval: std::time::Duration,
    /// Number of concurrent turn-runner slots (the scheduler semaphore permit
    /// count). `None` = unlimited — the semaphore is sized to
    /// [`tokio::sync::Semaphore::MAX_PERMITS`]. See [`scheduler_permit_count`].
    pub worker_count: Option<std::num::NonZeroUsize>,
    /// Capability IDs removed from every model-facing capability surface,
    /// regardless of the resolved capability-surface profile.
    pub disabled_capability_ids: Vec<CapabilityId>,
    pub text_only_driver: TextOnlyModelReplyDriverConfig,
    pub host: TextOnlyLoopHostConfig,
    pub tool_disclosure: ToolDisclosureMode,
    /// Rollout switch for bounded concurrent execution of batches whose
    /// capability descriptors all permit parallel execution.
    pub parallel_tool_batch: ParallelToolBatchMode,
    /// Profile-owned visibility preferences, keyed by capability-surface
    /// profile id. Values are canonical capability ids and never grant access.
    pub tool_disclosure_profile_pins: HashMap<CapabilitySurfaceProfileId, Vec<CapabilityId>>,
    pub planned_default_iteration_limit: Option<std::num::NonZeroU32>,
    /// Override for the default family's model availability-retry budget
    /// (`DefaultRecoveryStrategy::max_model_availability_attempts`). `None`
    /// keeps the production default; test harnesses set it low so scripted
    /// provider failures abort in seconds instead of riding out an outage.
    pub planned_model_availability_retry_attempts: Option<std::num::NonZeroU32>,
}

impl Default for DefaultPlannedRuntimeConfig {
    fn default() -> Self {
        Self {
            heartbeat_interval: std::time::Duration::from_secs(10),
            poll_interval: std::time::Duration::from_secs(5),
            lease_recovery_interval: std::time::Duration::from_secs(10),
            worker_count: Some(DEFAULT_TURN_RUNNER_WORKER_COUNT),
            disabled_capability_ids: default_disabled_capability_ids(),
            text_only_driver: TextOnlyModelReplyDriverConfig::default(),
            host: TextOnlyLoopHostConfig::default(),
            tool_disclosure: ToolDisclosureMode::from_env(),
            parallel_tool_batch: ParallelToolBatchMode::from_env(),
            tool_disclosure_profile_pins: HashMap::new(),
            planned_default_iteration_limit: None,
            planned_model_availability_retry_attempts: None,
        }
    }
}

pub const REBORN_TOOL_DISCLOSURE_PROFILE_PINS_ENV: &str = "REBORN_TOOL_DISCLOSURE_PROFILE_PINS";

impl DefaultPlannedRuntimeConfig {
    /// Resolve operator-controlled environment configuration for production
    /// startup. Programmatic `Default` remains deterministic; the composition
    /// root calls this fallible boundary so a malformed pin map cannot silently
    /// disable an intended optimization.
    pub fn try_from_env() -> Result<Self, DefaultPlannedRuntimeConfigError> {
        Ok(Self {
            tool_disclosure_profile_pins: tool_disclosure_profile_pins_from_env()?,
            ..Self::default()
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DefaultPlannedRuntimeConfigError {
    #[error("{REBORN_TOOL_DISCLOSURE_PROFILE_PINS_ENV} is not valid UTF-8")]
    ProfilePinsNotUnicode,
    #[error("{REBORN_TOOL_DISCLOSURE_PROFILE_PINS_ENV} is invalid: {source}")]
    ProfilePins {
        #[source]
        source: ToolDisclosureProfilePinsParseError,
    },
}

fn tool_disclosure_profile_pins_from_env()
-> Result<HashMap<CapabilitySurfaceProfileId, Vec<CapabilityId>>, DefaultPlannedRuntimeConfigError>
{
    let raw = match std::env::var(REBORN_TOOL_DISCLOSURE_PROFILE_PINS_ENV) {
        Ok(raw) => raw,
        Err(std::env::VarError::NotPresent) => return Ok(HashMap::new()),
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(DefaultPlannedRuntimeConfigError::ProfilePinsNotUnicode);
        }
    };
    parse_tool_disclosure_profile_pins(&raw)
        .map_err(|source| DefaultPlannedRuntimeConfigError::ProfilePins { source })
}

#[derive(Debug, thiserror::Error)]
pub enum ToolDisclosureProfilePinsParseError {
    #[error("profile-pin JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("profile id {profile:?} is invalid: {reason}")]
    Profile { profile: String, reason: String },
    #[error("capability id {capability:?} for profile {profile:?} is invalid: {reason}")]
    Capability {
        profile: String,
        capability: String,
        reason: String,
    },
}

fn parse_tool_disclosure_profile_pins(
    raw: &str,
) -> Result<
    HashMap<CapabilitySurfaceProfileId, Vec<CapabilityId>>,
    ToolDisclosureProfilePinsParseError,
> {
    let parsed = serde_json::from_str::<HashMap<String, Vec<String>>>(raw)?;
    let mut resolved = HashMap::new();
    for (profile, pins) in parsed {
        let profile_id = CapabilitySurfaceProfileId::new(profile.clone()).map_err(|reason| {
            ToolDisclosureProfilePinsParseError::Profile {
                profile: profile.clone(),
                reason,
            }
        })?;
        let pins = pins
            .into_iter()
            .map(|capability| {
                CapabilityId::new(capability.clone()).map_err(|error| {
                    ToolDisclosureProfilePinsParseError::Capability {
                        profile: profile.clone(),
                        capability,
                        reason: error.to_string(),
                    }
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        resolved.insert(profile_id, pins);
    }
    Ok(resolved)
}

/// Map the configured worker count into a scheduler-semaphore permit count.
///
/// `None` (the operator-selected "unlimited" sentinel, e.g.
/// `IRONCLAW_REBORN_RUNNER_WORKER_COUNT=0`) yields
/// [`tokio::sync::Semaphore::MAX_PERMITS`] so the global scheduler never
/// throttles claimed runs — the per-user / per-origin caps remain the only
/// concurrency bound. A bounded count is saturated at
/// [`tokio::sync::Semaphore::MAX_PERMITS`] — values above that ceiling are
/// clamped down rather than passed through, so this function never produces a
/// count that would panic `Semaphore::new` regardless of caller.
fn scheduler_permit_count(worker_count: Option<std::num::NonZeroUsize>) -> usize {
    worker_count
        .map(std::num::NonZeroUsize::get)
        // Saturate at tokio's ceiling: `Semaphore::new` panics ABOVE
        // `MAX_PERMITS`, and a request for more permits than that is already in
        // "no effective bound" territory (the `None` = unlimited path sizes the
        // semaphore to exactly `MAX_PERMITS`). This is a defense-in-depth
        // backstop for direct composition callers; the CLI layer still rejects
        // oversized operator config loudly before it ever reaches here.
        .unwrap_or(tokio::sync::Semaphore::MAX_PERMITS)
        .min(tokio::sync::Semaphore::MAX_PERMITS)
}

fn default_disabled_capability_ids() -> Vec<CapabilityId> {
    vec![
        CapabilityId::new(ironclaw_loop_host::DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID)
            .expect("static spawn_subagent capability id must be valid"), // safety: crate-owned static dotted id.
    ]
}

/// Opaque carrier for the scheduler's wake-pair (notifier + channel).
///
/// Keeps substrate types ([`SchedulerTurnRunWakeNotifier`], [`TurnRunWakeChannel`])
/// off public struct fields while still letting the production composition path
/// pre-mint the pair before building the coordinator (breaking the
/// coordinator↔scheduler build-order cycle).
///
/// The struct is `pub` so `ironclaw_composition` (the sanctioned downstream
/// consumer) can carry it through [`DefaultPlannedRuntimeParts`].  The raw
/// substrate types are not re-exposed: constructors and accessors only hand
/// back typed handles, never the bare fields.
pub struct SchedulerWakeWiring {
    notifier: Arc<SchedulerTurnRunWakeNotifier>,
    channel: TurnRunWakeChannel,
}

impl SchedulerWakeWiring {
    /// Mint a new notifier + paired wake channel using the default scheduler
    /// wake-channel capacity.
    pub fn channel() -> Self {
        let (notifier, channel) = SchedulerTurnRunWakeNotifier::channel(
            TurnRunSchedulerConfig::default().wake_channel_capacity(),
        );
        Self { notifier, channel }
    }

    /// Return a clone of the notifier so callers can register it with other
    /// services before the scheduler is started.
    pub fn notifier(&self) -> Arc<SchedulerTurnRunWakeNotifier> {
        Arc::clone(&self.notifier)
    }

    /// Start the scheduler loop, consuming the carrier.  Returns the handle
    /// so callers can query liveness and request shutdown.
    pub(crate) fn start(self, scheduler: TurnRunScheduler) -> TurnRunSchedulerHandle {
        scheduler.start_with_channel(self.notifier, self.channel)
    }
}

pub struct DefaultPlannedRuntimeParts<G>
where
    G: HostManagedModelGateway + ?Sized + Send + Sync + 'static,
{
    pub process_system: ProcessRuntimeSystem,
    pub thread_service: Arc<dyn SessionThreadService>,
    pub thread_scope: ThreadScope,
    pub model_gateway: Arc<G>,
    pub loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
    pub milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    pub capability_factory: Arc<dyn LoopCapabilityPortFactory>,
    pub capability_surface_resolver: Arc<dyn CapabilitySurfaceProfileResolver>,
    pub capability_result_writer: Arc<dyn LoopCapabilityResultWriter>,
    /// Await-edge writer, settlement, and evidence views over the process
    /// dependency journal.
    pub subagent_await_edge_writer: Arc<dyn AwaitEdgeWriter>,
    pub subagent_await_edge_settler: Arc<dyn AwaitEdgeSettler>,
    pub subagent_await_edge_evidence: Arc<dyn AwaitDependentRunEvidenceStore>,
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
    /// Process-local operator diagnostics captured at the resolved prompt boundary.
    pub prompt_diagnostic_sink: Option<Arc<dyn HostManagedPromptDiagnosticSink>>,
    /// Shared run-scoped intent store used by the explicit attachment
    /// capability and transcript finalizer. Production must pass the exact
    /// same handle to both sides so sealing cannot split from registration.
    pub reply_attachment_intent_port: Option<Arc<dyn ReplyAttachmentIntentPort>>,
    /// Durable store the loop-host persisted `GateRecord::Auth` into (§5.2.9),
    /// threaded to the turn executor so an auth block re-sources its
    /// `credential_requirements` from the host record (render-from-record) after
    /// the §5.3 flip moved them off the loop-facing channel. Must be the SAME
    /// `Arc` the composition wired into the capability port's
    /// `with_gate_record_store`, or the read scope/key will not find the saved
    /// record. `None` only for helper/test compositions with no run-state
    /// filesystem (they never raise a durable auth gate); a production `None` is
    /// a bug — the same "genuinely optional" shape as `attachment_read_port`.
    pub gate_record_store: Option<Arc<dyn ironclaw_approvals::GateRecordStorePort>>,
    pub input_queue: Option<Arc<dyn HostInputQueue>>,
    /// Terminal-reconciliation surface of the SAME queue as `input_queue`.
    /// When present, the composed [`ProcessTransitionPort`] is decorated with
    /// [`crate::steering_reconcile::SteeringReconcilingProcessTransitions`],
    /// so every terminal run transition (complete/fail/cancel) closes the
    /// run's steering queue, settles stranded `Queued` rows, and reclaims the
    /// per-run queue record. `None` only for compositions without a steering
    /// queue (then there is nothing to reconcile).
    pub input_queue_reconcile: Option<Arc<dyn HostInputQueueReconcile>>,
    /// Required by live planned-runtime composition. Helper-level tests may use
    /// a no-op implementation, but the type signature always requires a valid
    /// identity context source.
    pub identity_context_source: Arc<dyn HostIdentityContextSource>,
    /// Source for the per-user agent-context profile (timezone/locale/location).
    /// Resolved once at loop start and stamped into `LoopRuntimeContext.user_profile`.
    /// `EmptyUserProfileSource` (always `None`) is acceptable for compositions
    /// that do not yet wire a profile backend.
    pub user_profile_source: Arc<dyn HostUserProfileSource>,
    /// Proactive-memory source (#3537 / mem0 flow). Resolved once per run at the
    /// first prompt build and surfaced into the prompt's "memory" section.
    /// `None` is acceptable — and is the default for compositions whose memory
    /// binding is disabled or third-party-without-a-provider — degrading to no
    /// memory rather than failing the turn, the same optionality as
    /// `user_profile_source`.
    pub memory_context_service: Option<Arc<dyn MemoryPromptContextService>>,
    /// After-turn memory writer (#3537 / mem0 `add` flow). The RAW bound memory
    /// provider — the same `Arc<dyn MemoryService>` the memory tools resolve, NOT
    /// wrapped in a prompt-context adapter. When `Some`, the executor forwards each
    /// `Completed` run's full transcript to `record_interaction`, skipping only
    /// runs with no user/assistant content (the provider decides what to retain).
    /// `None` is acceptable — and is the default for compositions whose memory
    /// binding is disabled or third-party-without-a-provider — degrading to no
    /// after-turn recording rather than failing the turn (mirrors
    /// `memory_context_service`).
    pub after_turn_memory_writer: Option<Arc<dyn MemoryService>>,
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
    /// Pre-minted scheduler wake wiring.
    ///
    /// When `Some`, the inner build skips its own [`SchedulerWakeWiring::channel`] call
    /// and uses the provided carrier instead. This lets callers mint the notifier before
    /// composing the rest of the runtime (e.g. to satisfy a separate wiring validation gate)
    /// while still ensuring the scheduler loop consumes the exact same channel.
    ///
    /// When `None` (the default), the notifier and channel are minted internally, which is
    /// correct for standalone and any composition that does not need to pre-mint.
    pub scheduler_wake_wiring: Option<SchedulerWakeWiring>,
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
    pub scheduler_handle: TurnRunSchedulerHandle,
}

#[derive(Debug)]
pub enum DefaultPlannedRuntimeBuildError {
    DriverRegistry(DriverRegistryError),
    PlannedDriver(DefaultPlannedDriverRegistrationError),
    RunProfile(String),
    SubagentCompletion(String),
    SteeringReconcileObserver(String),
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
            Self::SteeringReconcileObserver(error) => {
                write!(
                    formatter,
                    "steering reconcile observer wiring failed: {error}"
                )
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
    let agent_turn_runtime: Arc<dyn AgentTurnRuntimePort> =
        Arc::new(parts.process_system.agent_turn_runtime());
    let await_dependent_run_evidence: Arc<dyn AwaitDependentRunEvidenceStore> =
        parts.subagent_await_edge_evidence.clone();
    parts.loop_exit_evidence = Arc::new(
        ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
            Arc::clone(&parts.thread_service),
            agent_turn_runtime,
            Arc::clone(&parts.loop_checkpoint_store),
            await_dependent_run_evidence,
            parts.thread_scope.clone(),
        )
        .with_cancellation_factory(cancellation_factory),
    );
    build_default_planned_runtime(parts).map_err(ProductLiveRuntimeBuildError::Runtime)
}

fn non_production_noop_safety_context() -> InstructionSafetyContext {
    tracing::debug!(
        "using standaloneelopment no-op instruction safety context; configure a real instruction safety scanner before product-live use"
    );
    InstructionSafetyContext::non_production_noop()
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
    build_default_planned_runtime_inner(parts)
}

fn build_default_planned_runtime_inner<G>(
    parts: DefaultPlannedRuntimeParts<G>,
) -> Result<
    RebornRuntimeLoopComposition<dyn SessionThreadService, G>,
    DefaultPlannedRuntimeBuildError,
>
where
    G: HostManagedModelGateway + ?Sized + Send + Sync + 'static,
{
    let mut registry = DriverRegistry::new();
    register_default_text_only_driver(&mut registry, parts.config.text_only_driver)?;
    let family_registry = build_loop_family_registry_with_overrides(
        parts.config.planned_default_iteration_limit,
        parts.config.planned_model_availability_retry_attempts,
        parts.config.parallel_tool_batch.enables_parallel_batches(),
    )
    .map_err(|error| {
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

    let process_system = parts.process_system.clone();
    let agent_turn_runtime = Arc::new(process_system.agent_turn_runtime());
    let cancellation_factory: Arc<dyn RunCancellationFactory> =
        parts.cancellation_factory.clone().unwrap_or_else(|| {
            Arc::new(AgentTurnRunCancellationFactory::new(
                agent_turn_runtime.clone() as Arc<dyn AgentTurnRuntimePort>,
            ))
        });

    // Resolve the scheduler wake wiring BEFORE building the coordinator, breaking
    // the coordinator↔scheduler build-order cycle.  The coordinator receives the
    // real notifier immediately; the channel is held in the carrier and passed to
    // `start_with_channel` via `SchedulerWakeWiring::start` after the executor is built.
    //
    // When a caller pre-minted the wiring (e.g. the production composition path
    // that must satisfy `HostRuntimeServices.with_turn_run_wake_notifier_dyn`
    // before this function runs), use it directly so the coordinator and the
    // scheduler share the exact same channel.  Otherwise mint a fresh carrier.
    let wake_wiring = parts
        .scheduler_wake_wiring
        .unwrap_or_else(SchedulerWakeWiring::channel);
    let scheduler_notifier_base: Arc<dyn TurnRunWakeNotifier> = wake_wiring.notifier();
    // Fan out each coordinator wake to BOTH the scheduler and the exact
    // cancellation factory installed on the loop host. Without this shared
    // instance, the scheduler still wakes but retained run handles never flip
    // on `cancel_run`.
    let wake_notifier: Arc<dyn TurnRunWakeNotifier> = Arc::new(CompositeTurnRunWakeNotifier::new(
        scheduler_notifier_base,
        Arc::clone(&cancellation_factory),
    ));
    let subagent_await_edge_settler = Arc::clone(&parts.subagent_await_edge_settler);
    subagent_await_edge_settler
        .bind_turn_tree_store(agent_turn_runtime.clone() as Arc<dyn AgentTurnSpawnTreeRuntimePort>)
        .map_err(|error| DefaultPlannedRuntimeBuildError::SubagentCompletion(error.to_string()))?;
    let subagent_completion_observer: Arc<dyn TurnCommittedEventObserver> =
        Arc::clone(&subagent_await_edge_settler).as_turn_committed_event_observer();
    process_system
        .subscribe_process_observer(Arc::new(AgentTurnProcessCommitObserver::new(
            subagent_completion_observer,
            parts.turn_event_sink.clone(),
        )))
        .map_err(DefaultPlannedRuntimeBuildError::SubagentCompletion)?;
    if let Some(reconcile) = parts.input_queue_reconcile.clone() {
        // Completeness net over the transition-port decoration below: the
        // scheduler/supervisor terminalizes crash-reclaimed and panicked runs
        // through the raw runtime handle, but every terminal transition lands
        // in the journal, and observer delivery is durable (cursor-tracked,
        // retried, replayed across restarts). Reconciliation is idempotent,
        // so double delivery with the decorator is a no-op.
        process_system
            .subscribe_process_observer(Arc::new(
                crate::steering_reconcile::SteeringReconcileCommitObserver::new(reconcile),
            ))
            .map_err(DefaultPlannedRuntimeBuildError::SteeringReconcileObserver)?;
    }
    let base_coordinator = DefaultTurnCoordinator::new(Arc::clone(&agent_turn_runtime))
        .with_run_profile_resolver(Arc::clone(&run_profile_resolver))
        .with_wake_notifier(Arc::clone(&wake_notifier))
        .with_process_runtime(agent_turn_runtime.as_ref().clone());
    let base_coordinator_arc = Arc::new(base_coordinator);
    let child_runs: Arc<dyn TurnSpawnTreePort> = base_coordinator_arc.clone();
    let coordinator: Arc<dyn ironclaw_turns::TurnCoordinator> = base_coordinator_arc;
    subagent_await_edge_settler
        .bind_coordinator(Arc::clone(&coordinator))
        .map_err(|error| DefaultPlannedRuntimeBuildError::SubagentCompletion(error.to_string()))?;

    // The host factory needs the spawn-tree superset: besides cancellation
    // observation it reads the claimed run's journal record to fence transcript
    // writes on the lease the run was actually claimed under.
    let agent_turn_runtime_port: Arc<dyn AgentTurnSpawnTreeRuntimePort> =
        agent_turn_runtime.clone();
    let subagent_prompt_source: Arc<dyn SubagentPromptMaterialSource> =
        Arc::new(GateBackedSubagentPromptMaterialSource::new(
            process_system.inputs(),
            Arc::clone(&parts.thread_service),
        ));
    let subagent_prompt_composer = SubagentPromptComposer::new(Arc::clone(&subagent_prompt_source));
    let spawn_decorator = Arc::new(SubagentSpawnCapabilityDecorator::new(
        SubagentSpawnDeps {
            coordinator: Arc::clone(&coordinator) as Arc<dyn ironclaw_turns::TurnCoordinator>,
            child_runs,
            agent_turn_runtime: agent_turn_runtime.clone()
                as Arc<dyn AgentTurnSpawnTreeRuntimePort>,
            thread_service: Arc::clone(&parts.thread_service),
            await_edge_writer: Arc::clone(&parts.subagent_await_edge_writer),
            definition_resolver: Arc::clone(&parts.subagent_definition_resolver),
            spawn_input_codec: Arc::clone(&parts.subagent_spawn_input_codec),
            result_writer: Arc::clone(&parts.capability_result_writer),
        },
        parts.subagent_spawn_limits,
        flavors::builtin_flavor_catalog(),
    )?);
    // Resolve once inside the runner-private factory below, then thread the
    // exact same Arc through disclosure and the outer profile filter.
    let capability_surface_resolver: Arc<dyn CapabilitySurfaceProfileResolver> =
        Arc::new(SubagentCapabilitySurfaceResolver::new(
            parts.capability_surface_resolver,
            Arc::clone(&subagent_prompt_source),
        ));
    let tool_disclosure_decorator = if parts.config.tool_disclosure.is_enabled() {
        tracing::debug!(
            target: "ironclaw::reborn::runtime",
            mode = ?parts.config.tool_disclosure,
            "reborn tool disclosure decorator wired"
        );
        Some(Arc::new(ToolDisclosureCapabilityDecorator::new(
            Arc::clone(&parts.capability_result_writer),
            parts.config.tool_disclosure,
        )))
    } else {
        None
    };
    // TEMP(disable-spawn-subagents): explicit composition decision to remove the
    // spawn_subagent capability from the model-facing surface across all
    // profiles. The ids are folded into the one resolved policy below, after
    // spawn decoration and before disclosure. Override `disabled_capability_ids`
    // to re-enable it in targeted regression harnesses.
    let global_denied = parts.config.disabled_capability_ids.clone();
    // Issue #5505: a scheduled-trigger fire must not be able to create,
    // remove, pause, or resume triggers (read-only trigger_list stays
    // available). These ids are folded into that run's one resolved policy only
    // for the `scheduled_trigger` capability-surface profile.
    let scheduled_trigger_denied = SCHEDULED_TRIGGER_DENIED_CAPABILITY_IDS
        .iter()
        .map(|id| CapabilityId::new(*id))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| DefaultPlannedRuntimeBuildError::RunProfile(error.to_string()))?;
    let capability_factory: Arc<dyn LoopCapabilityPortFactory> =
        Arc::new(RuntimeProfiledCapabilityPortFactory {
            inner: parts.capability_factory,
            surface_resolver: capability_surface_resolver,
            spawn_decorator,
            tool_disclosure_decorator,
            global_denied,
            scheduled_trigger_denied,
            tool_disclosure_profile_pins: parts.config.tool_disclosure_profile_pins,
        });
    let safety_context = parts
        .safety_context
        .unwrap_or_else(non_production_noop_safety_context);
    // Build the after-turn memory recorder before `parts.thread_scope` is moved
    // into the host factory below. Present only when a bound memory
    // provider was resolved; it owner-rewrites the base thread scope per run
    // before reading the just-finished exchange back.
    let after_turn_memory_recorder = parts.after_turn_memory_writer.clone().map(|memory_writer| {
        Arc::new(crate::after_turn_memory::AfterTurnMemoryRecorder::new(
            Arc::clone(&parts.thread_service),
            memory_writer,
            parts.thread_scope.clone(),
        ))
    });
    let mut host_factory = RebornLoopDriverHostFactory::new(
        Arc::clone(&parts.thread_service),
        parts.thread_scope,
        Arc::clone(&parts.model_gateway),
        agent_turn_runtime_port,
        Arc::clone(&parts.loop_checkpoint_store),
        parts.milestone_sink,
        parts.config.host,
        safety_context,
    )
    .with_resolved_profiled_capability_port_factory(capability_factory)
    .with_subagent_prompt_composer(subagent_prompt_composer)
    .with_driver_requirements(driver_registry.requirements_snapshot());
    if let Some(resolver) = parts.model_route_resolver {
        host_factory = host_factory.with_model_route_resolver(resolver);
    }
    host_factory = host_factory.with_cancellation_factory(cancellation_factory);
    if let Some(port) = parts.attachment_read_port {
        host_factory = host_factory.with_attachment_read_port(port);
    }
    if let Some(sink) = parts.prompt_diagnostic_sink {
        host_factory = host_factory.with_prompt_diagnostic_sink(sink);
    }
    if let Some(port) = parts.reply_attachment_intent_port {
        host_factory = host_factory.with_reply_attachment_intent_port(port);
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
    host_factory = host_factory.with_user_profile_source(parts.user_profile_source);
    if let Some(service) = parts.memory_context_service {
        host_factory = host_factory.with_memory_context_service(service);
    }
    let host_factory = Arc::new(host_factory);

    let mut process_transition_port: Arc<
        dyn ProcessTransitionPort<Error = ironclaw_turns::TurnError>,
    > = process_system.transitions();
    if let Some(reconcile) = parts.input_queue_reconcile {
        // Decorate the ONE transition port every terminal path flows through
        // (loop-exit apply, executor/scheduler failure fallbacks), so a run
        // that ends without a cancel still closes and reclaims its steering
        // queue.
        process_transition_port = Arc::new(
            crate::steering_reconcile::SteeringReconcilingProcessTransitions::new(
                process_transition_port,
                reconcile,
            ),
        );
    }
    let loop_exit_applier = Arc::new(LoopExitApplier::new(
        Arc::clone(&process_transition_port),
        parts.loop_exit_evidence,
    ));
    let mut executor = RebornTurnRunExecutor::new(
        Arc::clone(&loop_exit_applier),
        Arc::clone(&driver_registry),
        host_factory.clone() as Arc<dyn crate::turn_runner::HostFactory>,
        parts.gate_record_store.clone(),
    );
    if let Some(recorder) = after_turn_memory_recorder {
        executor = executor.with_after_turn_memory_recorder(recorder);
    }
    let executor = Arc::new(executor);
    let scheduler_config = TurnRunSchedulerConfig::default()
        .with_max_concurrent_runs(scheduler_permit_count(parts.config.worker_count))
        .with_runner_heartbeat_interval(parts.config.heartbeat_interval)
        .with_poll_interval(parts.config.poll_interval)
        .with_lease_recovery_interval(parts.config.lease_recovery_interval);
    let scheduler = TurnRunScheduler::new_with_process_runtime(
        process_system.runtime(),
        executor,
        scheduler_config,
    );
    let scheduler_handle = wake_wiring.start(scheduler);

    Ok(
        RebornRuntimeLoopComposition::<dyn SessionThreadService, G> {
            driver_registry,
            run_profile_resolver,
            coordinator,
            host_factory,
            scheduler_handle,
        },
    )
}

/// Issue #5505: a scheduled-trigger fire runs through the same agent loop as
/// an interactive turn, but must not be able to create/remove/pause/resume
/// triggers (a fire mutating the trigger fleet is exactly the reported "a
/// routine that creates routines" bug). Read-only
/// [`ironclaw_host_runtime::TRIGGER_LIST_CAPABILITY_ID`] is intentionally
/// excluded from this list. Folded into the one resolved host-API policy for
/// the
/// [`crate::planned_driver_factory::SCHEDULED_TRIGGER_CAPABILITY_SURFACE_PROFILE_ID`]
/// only.
const SCHEDULED_TRIGGER_DENIED_CAPABILITY_IDS: &[&str] = &[
    ironclaw_host_runtime::TRIGGER_CREATE_CAPABILITY_ID,
    ironclaw_host_runtime::TRIGGER_REMOVE_CAPABILITY_ID,
    ironclaw_host_runtime::TRIGGER_PAUSE_CAPABILITY_ID,
    ironclaw_host_runtime::TRIGGER_RESUME_CAPABILITY_ID,
];

/// Runner-private per-run factory that preserves the canonical wrapper order
/// while passing one resolved policy directly to every consumer that needs
/// it. The neutral loop-host decorator contract remains synchronous.
struct RuntimeProfiledCapabilityPortFactory {
    inner: Arc<dyn LoopCapabilityPortFactory>,
    surface_resolver: Arc<dyn CapabilitySurfaceProfileResolver>,
    spawn_decorator: Arc<dyn LoopCapabilityPortDecorator>,
    tool_disclosure_decorator: Option<Arc<ToolDisclosureCapabilityDecorator>>,
    global_denied: Vec<CapabilityId>,
    scheduled_trigger_denied: Vec<CapabilityId>,
    tool_disclosure_profile_pins: HashMap<CapabilitySurfaceProfileId, Vec<CapabilityId>>,
}

#[async_trait::async_trait]
impl LoopCapabilityPortFactory for RuntimeProfiledCapabilityPortFactory {
    async fn create_capability_port(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        let mut policy = self
            .surface_resolver
            .resolve(run_context)
            .await
            .map_err(capability_resolve_error_to_agent_host_error)?;
        let mut denied = self.global_denied.clone();
        if run_context
            .resolved_run_profile
            .capability_surface_profile_id
            .as_str()
            == crate::planned_driver_factory::SCHEDULED_TRIGGER_CAPABILITY_SURFACE_PROFILE_ID
        {
            denied.extend(self.scheduled_trigger_denied.iter().cloned());
        }
        policy = policy.deny_capability_ids(denied);
        let policy = Arc::new(policy);
        let mut capabilities = self
            .inner
            .create_capability_port_with_surface_policy(run_context, Arc::clone(&policy))
            .await?;
        capabilities = self.spawn_decorator.decorate(run_context, capabilities);
        capabilities = apply_capability_surface_policy(capabilities, Arc::clone(&policy));
        if let Some(decorator) = self.tool_disclosure_decorator.as_ref() {
            let pins = self
                .tool_disclosure_profile_pins
                .get(
                    &run_context
                        .resolved_run_profile
                        .capability_surface_profile_id,
                )
                .cloned()
                .unwrap_or_default();
            capabilities = decorator.decorate_with_policy_and_pins(
                run_context,
                capabilities,
                Arc::clone(&policy),
                pins,
            );
        }
        Ok(capabilities)
    }
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
        let spawn_id = CapabilityId::new(ironclaw_loop_host::DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID)
            .map_err(|error| DefaultPlannedRuntimeBuildError::RunProfile(error.to_string()))?;
        let parameters_schema = Arc::new(
            ironclaw_loop_host::build_spawn_subagent_parameters_schema(&flavor_catalog),
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
    use std::collections::HashMap;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use super::{
        DefaultPlannedRuntimeConfig, ParallelToolBatchMode,
        REBORN_TOOL_DISCLOSURE_PROFILE_PINS_ENV, RuntimeProfiledCapabilityPortFactory,
        SCHEDULED_TRIGGER_DENIED_CAPABILITY_IDS, ToolDisclosureCapabilityDecorator,
        ToolDisclosureMode, parse_tool_disclosure_profile_pins, scheduler_permit_count,
    };
    use async_trait::async_trait;
    use ironclaw_host_api::{
        capability_surface::CapabilitySurfacePolicy,
        ids::{AgentId, CapabilityId, ProjectId, TenantId, ThreadId},
        resolution::{Resolution, ResolutionBatch},
        runtime::RuntimeKind,
    };
    use ironclaw_host_runtime::{
        TRIGGER_CREATE_CAPABILITY_ID, TRIGGER_LIST_CAPABILITY_ID, TRIGGER_PAUSE_CAPABILITY_ID,
        TRIGGER_REMOVE_CAPABILITY_ID, TRIGGER_RESUME_CAPABILITY_ID,
    };
    use ironclaw_loop_contracts::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityDescriptorView,
        CapabilitySurfaceProfileId, CapabilitySurfaceVersion, ConcurrencyHint,
        InMemoryRunProfileResolver, LoopCapabilityPort, LoopRequest, LoopRequestBatch,
        LoopRunContext, RunProfileResolutionRequest, RunProfileResolver, VisibleCapabilityRequest,
        VisibleCapabilitySurface,
    };
    use ironclaw_turns::{TurnId, TurnRunId, TurnScope};

    use ironclaw_loop_host::{
        CapabilityResolveError, CapabilitySurfaceProfileResolver,
        DecoratingLoopCapabilityPortFactory, LoopCapabilityPortDecorator,
        LoopCapabilityPortFactory,
    };

    #[test]
    fn parallel_tool_batch_mode_fails_closed_to_off() {
        assert_eq!(
            ParallelToolBatchMode::from_raw(None),
            ParallelToolBatchMode::Off
        );
        assert_eq!(
            ParallelToolBatchMode::from_raw(Some("")),
            ParallelToolBatchMode::Off
        );
        assert_eq!(
            ParallelToolBatchMode::from_raw(Some("garbage")),
            ParallelToolBatchMode::Off
        );
        assert_eq!(
            ParallelToolBatchMode::from_raw(Some("off")),
            ParallelToolBatchMode::Off
        );
        assert!(!ParallelToolBatchMode::Off.enables_parallel_batches());
    }

    #[test]
    fn parallel_tool_batch_mode_accepts_explicit_on_values() {
        for raw in ["on", "ON", "1", "true", "TRUE"] {
            assert_eq!(
                ParallelToolBatchMode::from_raw(Some(raw)),
                ParallelToolBatchMode::On
            );
        }
        assert!(ParallelToolBatchMode::On.enables_parallel_batches());
    }

    #[test]
    fn scheduler_permit_count_unlimited_uses_max_permits() {
        // `None` is the "no global throttle" sentinel — the scheduler semaphore
        // must be sized to the largest value tokio accepts, and `Semaphore::new`
        // must not panic on it.
        let permits = scheduler_permit_count(None);
        assert_eq!(permits, tokio::sync::Semaphore::MAX_PERMITS);
        // Constructing the semaphore with this count must not panic.
        let _ = tokio::sync::Semaphore::new(permits);
    }

    #[test]
    fn scheduler_permit_count_bounded_passes_through() {
        let permits =
            scheduler_permit_count(Some(std::num::NonZeroUsize::new(7).expect("non-zero")));
        assert_eq!(permits, 7);
    }

    #[test]
    fn scheduler_permit_count_saturates_above_max_permits() {
        // A direct composition caller passing more permits than tokio accepts must
        // not panic the scheduler; it saturates to the ceiling instead.
        let over =
            std::num::NonZeroUsize::new(tokio::sync::Semaphore::MAX_PERMITS + 1).expect("non-zero");
        let permits = scheduler_permit_count(Some(over));
        assert_eq!(permits, tokio::sync::Semaphore::MAX_PERMITS);
        // Must not panic.
        let _ = tokio::sync::Semaphore::new(permits);
    }

    #[test]
    fn profile_pin_config_parses_typed_capability_ids() {
        let parsed = parse_tool_disclosure_profile_pins(
            r#"{"interactive_tools":["github.search_code","gmail.list_messages"]}"#,
        )
        .expect("valid profile-pin configuration");

        assert_eq!(
            parsed[&CapabilitySurfaceProfileId::new("interactive_tools")
                .expect("valid test profile id")]
                .iter()
                .map(CapabilityId::as_str)
                .collect::<Vec<_>>(),
            vec!["github.search_code", "gmail.list_messages"]
        );
    }

    #[test]
    fn profile_pin_config_rejects_the_entire_map_when_any_id_is_invalid() {
        assert!(
            parse_tool_disclosure_profile_pins(
                r#"{"interactive_tools":["github.search_code"],"mission":["invalid"]}"#,
            )
            .is_err()
        );
        assert!(parse_tool_disclosure_profile_pins("[]").is_err());
        assert!(
            parse_tool_disclosure_profile_pins(r#"{"INVALID PROFILE":["github.search_code"]}"#,)
                .is_err(),
            "profile identities must be validated at the environment boundary"
        );
    }

    #[test]
    fn profile_pin_environment_rejects_invalid_configuration_at_runtime_startup() {
        const CHILD_MARKER: &str = "IRONCLAW_PROFILE_PIN_CONFIG_TEST_CHILD";
        if std::env::var_os(CHILD_MARKER).is_some() {
            let error = DefaultPlannedRuntimeConfig::try_from_env()
                .expect_err("invalid profile pins must reject runtime configuration");
            assert!(
                error.to_string().contains("is invalid"),
                "startup error must retain configuration context: {error}"
            );
            return;
        }

        for invalid in ["{", r#"{"interactive_tools":["invalid"]}"#] {
            let output = std::process::Command::new(
                std::env::current_exe().expect("current test executable"),
            )
            .args([
                "profile_pin_environment_rejects_invalid_configuration_at_runtime_startup",
                "--nocapture",
            ])
            .env(CHILD_MARKER, "1")
            .env(REBORN_TOOL_DISCLOSURE_PROFILE_PINS_ENV, invalid)
            .output()
            .expect("child test process executes");
            assert!(
                output.status.success(),
                "runtime startup must reject invalid pin config: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    async fn test_run_context() -> LoopRunContext {
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .unwrap();
        test_run_context_with_resolved_profile(resolved)
    }

    fn test_run_context_with_resolved_profile(
        resolved: ironclaw_loop_contracts::ResolvedRunProfile,
    ) -> LoopRunContext {
        let tenant_id = TenantId::new("tenant-runtime-test").unwrap();
        let agent_id = AgentId::new("agent-runtime-test").unwrap();
        let project_id = ProjectId::new("project-runtime-test").unwrap();
        let thread_id = ThreadId::new("thread-runtime-test").unwrap();
        let turn_scope = TurnScope::new(tenant_id, Some(agent_id), Some(project_id), thread_id);
        LoopRunContext::new(turn_scope, TurnId::new(), TurnRunId::new(), resolved)
    }

    /// Run context whose resolved profile carries the `scheduled_trigger`
    /// capability-surface-profile id — built through the real production
    /// resolver (`default_planned_run_profile_resolver`), not a hand-rolled
    /// `ResolvedRunProfile`, so the test tracks the actual registration.
    async fn scheduled_trigger_run_context() -> LoopRunContext {
        use crate::planned_driver_factory::default_planned_run_profile_resolver;
        use ironclaw_turns::{RunProfileId, RunProfileRequest};

        let resolver =
            default_planned_run_profile_resolver().expect("planned resolver should build");
        let resolved = resolver
            .resolve_run_profile(
                RunProfileResolutionRequest::interactive_default().with_requested_run_profile(
                    RunProfileRequest::new(RunProfileId::scheduled_trigger().as_str()).unwrap(),
                ),
            )
            .await
            .expect("scheduled_trigger profile should resolve");
        test_run_context_with_resolved_profile(resolved)
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
            _request: LoopRequest,
        ) -> Result<Resolution, AgentLoopHostError> {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                format!("{label} unused", label = self.label),
            ))
        }

        async fn invoke_capability_batch(
            &self,
            _request: LoopRequestBatch,
        ) -> Result<ResolutionBatch, AgentLoopHostError> {
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
            request: LoopRequest,
        ) -> Result<Resolution, AgentLoopHostError> {
            self.log.lock().unwrap().push(self.label);
            self.inner.invoke_capability(request).await
        }

        async fn invoke_capability_batch(
            &self,
            request: LoopRequestBatch,
        ) -> Result<ResolutionBatch, AgentLoopHostError> {
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

    struct RecordingSurfacePolicyFactory {
        port: Arc<dyn LoopCapabilityPort>,
        policies: Arc<Mutex<Vec<CapabilitySurfacePolicy>>>,
    }

    #[async_trait]
    impl LoopCapabilityPortFactory for RecordingSurfacePolicyFactory {
        async fn create_capability_port(
            &self,
            _run_context: &LoopRunContext,
        ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
            Ok(Arc::clone(&self.port))
        }

        async fn create_capability_port_with_surface_policy(
            &self,
            _run_context: &LoopRunContext,
            surface_policy: Arc<CapabilitySurfacePolicy>,
        ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
            self.policies
                .lock()
                .unwrap()
                .push(surface_policy.as_ref().clone());
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

    struct CountingSurfaceResolver {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl CapabilitySurfaceProfileResolver for CountingSurfaceResolver {
        async fn resolve(
            &self,
            _run_context: &LoopRunContext,
        ) -> Result<CapabilitySurfacePolicy, CapabilityResolveError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(CapabilitySurfacePolicy::allow_all())
        }
    }

    struct UnusedResultWriter;

    #[async_trait]
    impl ironclaw_loop_host::LoopCapabilityResultWriter for UnusedResultWriter {
        async fn write_capability_result(
            &self,
            _write: ironclaw_loop_host::CapabilityResultWrite<'_>,
        ) -> Result<ironclaw_loop_host::CapabilityWriteResult, AgentLoopHostError> {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "unused in profile-resolution test",
            ))
        }
    }

    #[tokio::test]
    async fn profiled_factory_resolves_policy_once_with_tool_disclosure() {
        let calls = Arc::new(AtomicUsize::new(0));
        let decorate_calls = Arc::new(AtomicUsize::new(0));
        let policies = Arc::new(Mutex::new(Vec::new()));
        let denied_id = CapabilityId::new("demo.denied").expect("valid capability id");
        let inner = Arc::new(InnerPort {
            label: "inner",
            log: Arc::new(Mutex::new(Vec::new())),
        });
        let factory = RuntimeProfiledCapabilityPortFactory {
            inner: Arc::new(RecordingSurfacePolicyFactory {
                port: inner,
                policies: Arc::clone(&policies),
            }),
            surface_resolver: Arc::new(CountingSurfaceResolver {
                calls: Arc::clone(&calls),
            }),
            spawn_decorator: Arc::new(NoopDecorator {
                decorate_calls: Arc::clone(&decorate_calls),
            }),
            tool_disclosure_decorator: Some(Arc::new(ToolDisclosureCapabilityDecorator::new(
                Arc::new(UnusedResultWriter),
                ToolDisclosureMode::Bridged,
            ))),
            global_denied: vec![denied_id.clone()],
            scheduled_trigger_denied: Vec::new(),
            tool_disclosure_profile_pins: HashMap::new(),
        };

        factory
            .create_capability_port(&test_run_context().await)
            .await
            .expect("profiled capability port should build");

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "one resolved policy must be shared by disclosure and enforcement"
        );
        let policies = policies.lock().unwrap();
        assert_eq!(
            policies.len(),
            1,
            "the resolved policy is passed inward once"
        );
        assert!(
            !policies[0].permits_capability_id(&denied_id),
            "the host-visible request must receive the final deny-subtracted policy"
        );
    }

    // ── Issue #5505: scheduled-trigger capability-surface deny-map ───────────

    /// Fixed-surface inner port standing in for the host capability port.
    struct FixedSurfacePort {
        surface: VisibleCapabilitySurface,
    }

    #[async_trait]
    impl LoopCapabilityPort for FixedSurfacePort {
        async fn visible_capabilities(
            &self,
            _request: VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
            Ok(self.surface.clone())
        }

        async fn invoke_capability(
            &self,
            _request: LoopRequest,
        ) -> Result<Resolution, AgentLoopHostError> {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "unused in this test",
            ))
        }

        async fn invoke_capability_batch(
            &self,
            _request: LoopRequestBatch,
        ) -> Result<ResolutionBatch, AgentLoopHostError> {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "unused in this test",
            ))
        }
    }

    fn descriptor(capability_id: &str) -> CapabilityDescriptorView {
        CapabilityDescriptorView {
            capability_id: CapabilityId::new(capability_id).expect("test capability id is valid"),
            provider: None,
            runtime: RuntimeKind::Wasm,
            safe_name: capability_id.to_string(),
            safe_description: format!("{capability_id} description"),
            description_trust: Default::default(),
            concurrency_hint: ConcurrencyHint::SafeForParallel,
            parameters_schema: serde_json::json!({"type": "object"}),
        }
    }

    fn full_trigger_and_spawn_surface() -> VisibleCapabilitySurface {
        VisibleCapabilitySurface {
            version: CapabilitySurfaceVersion::new("surface-v1").expect("test version is valid"),
            descriptors: vec![
                descriptor(ironclaw_loop_host::DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID),
                descriptor(TRIGGER_CREATE_CAPABILITY_ID),
                descriptor(TRIGGER_LIST_CAPABILITY_ID),
                descriptor(TRIGGER_REMOVE_CAPABILITY_ID),
                descriptor(TRIGGER_PAUSE_CAPABILITY_ID),
                descriptor(TRIGGER_RESUME_CAPABILITY_ID),
                descriptor("builtin.echo"),
            ],
            callable_capability_ids: None,
        }
    }

    /// Derives the test-driven mutator id list directly from the production
    /// `SCHEDULED_TRIGGER_DENIED_CAPABILITY_IDS` constant rather than
    /// re-listing the four capability ids by name. Hand-duplicating the list
    /// here would let the unit tests below keep passing even if the
    /// production const accidentally dropped one of the mutators — deriving
    /// from the const closes that drift risk (PR #5515 review comment).
    fn scheduled_trigger_mutator_ids() -> Vec<CapabilityId> {
        SCHEDULED_TRIGGER_DENIED_CAPABILITY_IDS
            .iter()
            .map(|id| CapabilityId::new(*id).expect("trigger mutator capability id is valid"))
            .collect()
    }

    async fn visible_capability_ids(
        factory: &dyn LoopCapabilityPortFactory,
        run_context: &LoopRunContext,
    ) -> Vec<String> {
        let port = factory
            .create_capability_port(run_context)
            .await
            .expect("composed capability port");
        port.visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible capabilities")
            .descriptors
            .into_iter()
            .map(|descriptor| descriptor.capability_id.as_str().to_string())
            .collect()
    }

    #[tokio::test]
    async fn scheduled_trigger_surface_excludes_mutators_interactive_includes_them() {
        // Test through the caller: drive the same runner-private factory that
        // resolves and enforces the production policy.
        let global_denied = vec![
            CapabilityId::new(ironclaw_loop_host::DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).unwrap(),
        ];
        let inner: Arc<dyn LoopCapabilityPort> = Arc::new(FixedSurfacePort {
            surface: full_trigger_and_spawn_surface(),
        });
        let factory = RuntimeProfiledCapabilityPortFactory {
            inner: Arc::new(StaticFactory { port: inner }),
            surface_resolver: Arc::new(CountingSurfaceResolver {
                calls: Arc::new(AtomicUsize::new(0)),
            }),
            spawn_decorator: Arc::new(NoopDecorator {
                decorate_calls: Arc::new(AtomicUsize::new(0)),
            }),
            tool_disclosure_decorator: None,
            global_denied,
            scheduled_trigger_denied: scheduled_trigger_mutator_ids(),
            tool_disclosure_profile_pins: HashMap::new(),
        };

        let scheduled_ids =
            visible_capability_ids(&factory, &scheduled_trigger_run_context().await).await;
        assert!(
            !scheduled_ids.contains(&TRIGGER_CREATE_CAPABILITY_ID.to_string()),
            "scheduled_trigger surface must exclude trigger_create: {scheduled_ids:?}"
        );
        assert!(!scheduled_ids.contains(&TRIGGER_REMOVE_CAPABILITY_ID.to_string()));
        assert!(!scheduled_ids.contains(&TRIGGER_PAUSE_CAPABILITY_ID.to_string()));
        assert!(!scheduled_ids.contains(&TRIGGER_RESUME_CAPABILITY_ID.to_string()));
        assert!(
            scheduled_ids.contains(&TRIGGER_LIST_CAPABILITY_ID.to_string()),
            "read-only trigger_list must remain visible on scheduled_trigger surface: {scheduled_ids:?}"
        );
        assert!(
            !scheduled_ids
                .contains(&ironclaw_loop_host::DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID.to_string()),
            "global deny list must still apply on scheduled_trigger surface"
        );

        let interactive_ids = visible_capability_ids(&factory, &test_run_context().await).await;
        assert!(
            interactive_ids.contains(&TRIGGER_CREATE_CAPABILITY_ID.to_string()),
            "interactive_tools surface must include trigger_create: {interactive_ids:?}"
        );
        assert!(interactive_ids.contains(&TRIGGER_REMOVE_CAPABILITY_ID.to_string()));
        assert!(interactive_ids.contains(&TRIGGER_PAUSE_CAPABILITY_ID.to_string()));
        assert!(interactive_ids.contains(&TRIGGER_RESUME_CAPABILITY_ID.to_string()));
        assert!(interactive_ids.contains(&TRIGGER_LIST_CAPABILITY_ID.to_string()));
        assert!(
            !interactive_ids
                .contains(&ironclaw_loop_host::DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID.to_string()),
            "global deny list must still apply on the interactive surface"
        );
    }

    #[tokio::test]
    async fn scheduled_trigger_mutators_stay_denied_when_global_deny_list_is_emptied() {
        // Regression for the footgun described in the plan: emptying
        // the global disabled-capability list (the documented spawn_subagent
        // re-enable toggle) must never silently re-enable the trigger mutators
        // for a scheduled fire.
        let inner: Arc<dyn LoopCapabilityPort> = Arc::new(FixedSurfacePort {
            surface: full_trigger_and_spawn_surface(),
        });
        let factory = RuntimeProfiledCapabilityPortFactory {
            inner: Arc::new(StaticFactory { port: inner }),
            surface_resolver: Arc::new(CountingSurfaceResolver {
                calls: Arc::new(AtomicUsize::new(0)),
            }),
            spawn_decorator: Arc::new(NoopDecorator {
                decorate_calls: Arc::new(AtomicUsize::new(0)),
            }),
            tool_disclosure_decorator: None,
            global_denied: Vec::new(),
            scheduled_trigger_denied: scheduled_trigger_mutator_ids(),
            tool_disclosure_profile_pins: HashMap::new(),
        };

        let scheduled_ids =
            visible_capability_ids(&factory, &scheduled_trigger_run_context().await).await;

        assert!(!scheduled_ids.contains(&TRIGGER_CREATE_CAPABILITY_ID.to_string()));
        assert!(!scheduled_ids.contains(&TRIGGER_REMOVE_CAPABILITY_ID.to_string()));
        assert!(!scheduled_ids.contains(&TRIGGER_PAUSE_CAPABILITY_ID.to_string()));
        assert!(!scheduled_ids.contains(&TRIGGER_RESUME_CAPABILITY_ID.to_string()));
        assert!(scheduled_ids.contains(&TRIGGER_LIST_CAPABILITY_ID.to_string()));
        // Global list was empty, so the previously-denied spawn_subagent
        // capability is now visible again (expected — this is what "emptying
        // the toggle" means); only the scheduled-trigger set stays denied.
        assert!(
            scheduled_ids
                .contains(&ironclaw_loop_host::DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID.to_string())
        );
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
        // constraint. The companion empty-catalog test (gap 1, loop_host)
        // confirms the absent-enum guard on the other side.
        use ironclaw_loop_host::build_spawn_subagent_parameters_schema;

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
