//! Assembled Reborn runtime: substrate + drivers + worker, started as one.
//!
//! This module is the "later slice" the crate-level docstring promises:
//! product-level wiring on top of the substrate facades exposed by
//! `build_reborn_services`. It is the **only** place in the workspace where
//! `ironclaw_runner` (drivers, host factory, model gateway bridge),
//! `ironclaw_threads` (session thread service), and `ironclaw_llm` are
//! composed into a running agent.
//!
//! Downstream callers (the CLI, future channel adapters, e2e harnesses) reach
//! this assembly only through:
//!
//! - [`build_reborn_runtime`] — construct + start the runtime
//! - [`RebornRuntime`] — task-level handle (`new_conversation`,
//!   `send_user_message`, `shutdown`)
//!
//! They never name the underlying `TurnCoordinator`, `SessionThreadService`,
//! `LoopExitApplier`, `HostManagedModelGateway`, etc. directly. That is the
//! property that satisfies the "narrow Reborn public surface" requirement
//! pinned by `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs`.

// arch-exempt: large_file, needs Reborn runtime helper extraction, plan #4471
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use thiserror::Error;
use tokio::sync::{Mutex, OwnedMutexGuard};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use ironclaw_events::{DurableAuditLog, DurableEventLog, InMemoryAuditSink, RuntimeEvent};
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::RootFilesystem;
use ironclaw_first_party_extension_ports::{
    FirstPartySkillsExtension, FirstPartySkillsExtensionHandles, SelectableSkillContextSource,
    SkillActivationSelectorConfig, SkillExecutionAdapter, SkillInjectionMode,
};
use ironclaw_host_api::{
    ActionResultSummary, ActionSummary, AgentId, ApprovalRequestId, AuditEnvelope, AuditEventId,
    AuditStage, CapabilityId, CorrelationId, DecisionSummary, EffectKind, ExtensionId,
    InvocationId, Principal, ProjectId, ResourceScope, TenantId, ThreadId, UserId,
};
use ironclaw_loop_host::{
    AwaitEdgeSettler, AwaitEdgeWriter, CapabilityAllowSet, CapabilityResolveError,
    CapabilitySurfaceProfileResolver, EmptyUserProfileSource, FilesystemSkillBundleSource,
    HostIdentityContextSource, HostSkillContextSource, HostUserProfileSource,
    JsonSpawnSubagentInputCodec, LoopCapabilityInputResolver, LoopCapabilityPortFactory,
    LoopCapabilityResultWriter, ModelGatewayBackedSystemInferencePort,
};
use ironclaw_observability::live_latency_started_at;
use ironclaw_product_adapters::ProjectionStream;
use ironclaw_product_workflow::{
    ApprovalBlockedTurnRun, ApprovalInteractionScope, ApprovalInteractionService,
    ApprovalResolverPort, ApprovalTurnRunLocator, AuthInteractionService,
    DefaultApprovalInteractionService, DefaultAuthInteractionService,
    OutboundPreferencesProductFacade, PersistentApprovalGranteeResolver,
    RunStateApprovalInteractionReadModel,
};
use ironclaw_runner::loop_exit_applier::{
    ApprovalGateEvidenceStore, AwaitDependentRunEvidenceStore, ThreadCheckpointLoopExitEvidencePort,
};
use ironclaw_runner::milestone_events::{
    DurableLoopHostMilestoneScope, DurableLoopHostMilestoneSink,
};
use ironclaw_runner::runtime::{
    DefaultPlannedRuntimeBuildError, DefaultPlannedRuntimeConfig, DefaultPlannedRuntimeParts,
    RuntimeSubagentGoalStore, RuntimeTurnStateStore, ToolDisclosureMode,
    build_default_planned_runtime,
};
use ironclaw_runner::subagent::await_edge::{
    boot_recovery::ScopeRecoveryDriver, resolver::AwaitEdgeResolver,
    store::FilesystemAwaitEdgeStore,
};
use ironclaw_runner::subagent::flavors::StaticSubagentDefinitionResolver;
use ironclaw_runner::subagent::goal_store::FilesystemSubagentGoalStore;
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, MessageContent, MessageKind, MessageStatus,
    SessionThreadService, ThreadHistoryRequest, ThreadScope,
};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, GetRunStateRequest, IdempotencyKey,
    LoopGateRef, ReplyTargetBindingRef, RunProfileResolutionRequest, SanitizedCancelReason,
    SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCoordinator, TurnError,
    TurnEventProjectionSource, TurnId, TurnPersistenceSnapshot, TurnRunId, TurnRunRecord,
    TurnRunState, TurnRunWake, TurnScope, TurnSpawnTreeStateStore, TurnStateStoreLimits,
    TurnStatus,
    events::EventCursor,
    run_profile::{LoopHostMilestoneSink, LoopRunContext},
};

use ironclaw_host_runtime::MemoryBackedUserProfileSource;
#[cfg(any(test, feature = "test-support"))]
use ironclaw_outbound::OutboundError;
#[cfg(any(test, feature = "test-support"))]
use ironclaw_product_workflow::RebornOutboundDeliveryTargetId;
use ironclaw_turns::run_profile::UserProfileContext;

use self::latency::{trace_runtime_latency_error, trace_runtime_latency_ok};
use self::runtime_turn_scheduler::RuntimeTurnScheduler;
use crate::builtin_capability_policy::{BuiltinCapabilityPolicy, builtin_capability_policy};
use crate::deployment::{DeploymentConfig, RuntimeSubstrate, TrafficPolicy};
use crate::factory::{ComposedTurnStateStore, builtin_extension_registry};
#[cfg(any(test, feature = "test-support"))]
use crate::outbound::OutboundDeliveryTargetRegistrationOutcome;
#[cfg(any(test, feature = "test-support"))]
use crate::outbound::{
    DeliveryTargetCapabilities, OutboundDeliveryTargetEntry, OutboundDeliveryTargetId,
    OutboundDeliveryTargetOwner, OutboundDeliveryTargetScope, OutboundDeliveryTargetSummary,
};
use crate::outbound::{
    MutableOutboundDeliveryTargetRegistry, OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID,
    OutboundDeliveryTargetProvider, RebornOutboundPreferencesFacade,
    outbound_delivery_synthetic_provider,
};
use crate::projection::{RebornProjectionServices, build_reborn_projection_services};
use crate::root::default_system_prompt::DefaultSystemPromptIdentitySource;
use crate::turn_run_snapshot::TurnRunSnapshotSource;
use ironclaw_filesystem::CompositeRootFilesystem;

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone)]
struct StaticOutboundDeliveryTargetProvider {
    summary: OutboundDeliveryTargetSummary,
    capabilities: DeliveryTargetCapabilities,
    reply_target_binding_ref: ReplyTargetBindingRef,
}

#[cfg(any(test, feature = "test-support"))]
#[async_trait::async_trait]
impl OutboundDeliveryTargetProvider for StaticOutboundDeliveryTargetProvider {
    async fn list_outbound_delivery_targets(
        &self,
        caller: &OutboundDeliveryTargetScope,
    ) -> Result<Vec<OutboundDeliveryTargetEntry>, OutboundError> {
        // Static test/QA fixture available to whichever caller asks: it claims
        // the querying caller as owner so it always survives the registry's
        // caller-scoping filter. Real providers derive the owner from the
        // resolved resource instead.
        Ok(vec![OutboundDeliveryTargetEntry {
            summary: self.summary.clone(),
            capabilities: self.capabilities.clone(),
            reply_target_binding_ref: self.reply_target_binding_ref.clone(),
            owner: OutboundDeliveryTargetOwner::for_scope(caller),
        }])
    }
}
use crate::RebornCompositionProfile;
#[cfg(any(test, feature = "test-support"))]
use crate::automation::trigger_poller::TenantScopedTrustedTriggerFireAuthorizer;
use crate::automation::trigger_poller::{
    AccessCheckerTriggerFireAuthorizer, ConversationContentRefMaterializer,
    SnapshotActiveRunLookup, TRIGGER_POLLER_SHUTDOWN_TIMEOUT, TriggerPollerCompositionDeps,
    TriggerPollerRuntimeHandle, spawn_trigger_poller,
};
use crate::runtime_input::{
    PollSettings, RebornRuntimeIdentity, RebornRuntimeInput, TriggerFireAccessChecker,
    TriggerFireAccessGrant, TriggerPollerAuthorizerConfig, TriggerPollerSettings,
};
use crate::trigger_fire_access::{
    CompositeTriggerFireChecker, IdentityMembershipTriggerFireChecker,
    StaticOwnerTriggerFireChecker,
};
use crate::{
    RebornBuildError, RebornProductAuthServices, RebornReadiness, RebornServices,
    build_reborn_services,
};
use production::{
    EmptyCapabilitySurfaceResolver, EmptyIdentityContextSource,
    UnavailableApprovalInteractionService, UnavailableCapabilityIo,
    UnavailableCapabilityPortFactory,
};

const MAX_DESCENDANT_CANCEL_NODES: usize = 1_000;

// Adapter: wraps `MemoryBackedUserProfileSource` (in `ironclaw_host_runtime`) and
// implements `HostUserProfileSource` (in `ironclaw_loop_host`). A direct
// `impl HostUserProfileSource for MemoryBackedUserProfileSource` is forbidden by
// the orphan rule — neither the trait nor the type is defined in this crate. The
// newtype wrapper is defined here, so the impl is allowed. This mirrors how
// `WorkspaceIdentityContextSource` (defined in `src/workspace/`) implements
// `HostIdentityContextSource` (defined in `ironclaw_loop_host`) — the impl
// lives in the crate that owns the *concrete type* and can see the trait.
//
// `pub(crate)` so the `test_support::build_user_profile_source_for_test`
// forwarder can reuse this single adapter instead of duplicating the orphan-rule
// workaround in the test harness.
pub(crate) struct MemoryBackedUserProfileSourceAdapter(pub(crate) MemoryBackedUserProfileSource);

#[async_trait::async_trait]
impl HostUserProfileSource for MemoryBackedUserProfileSourceAdapter {
    async fn resolve_user_profile(
        &self,
        run_context: &LoopRunContext,
    ) -> Option<UserProfileContext> {
        // Delegate to the inherent method on `MemoryBackedUserProfileSource`.
        self.0.resolve_user_profile(run_context).await
    }
}

struct RuntimeStoreParts<'a> {
    local_runtime: Option<&'a crate::factory::RebornRuntimeSubstrate>,
    turn_state_store: Arc<dyn RuntimeTurnStateStore>,
    checkpoint_state_store: Arc<dyn ironclaw_turns::CheckpointStateStore>,
    loop_checkpoint_store: Arc<dyn ironclaw_turns::LoopCheckpointStore>,
    thread_service: Arc<dyn SessionThreadService>,
    event_log: Arc<dyn DurableEventLog>,
    audit_log: Arc<dyn DurableAuditLog>,
    resource_governor: Arc<dyn ironclaw_resources::ResourceGovernor>,
    budget_gate_store: Arc<dyn ironclaw_resources::BudgetGateStore>,
    broadcast_budget_event_sink: Arc<ironclaw_resources::BroadcastBudgetEventSink>,
    subagent_goal_store: Arc<dyn RuntimeSubagentGoalStore>,
    /// §3 replacement for `subagent_gate_store`: built here (not later, once
    /// `capability_result_writer` becomes available) because `F` (the
    /// filesystem backend generic) is only nameable inside
    /// `local_runtime_parts`/`production_runtime_parts` — by the time the
    /// shared caller destructures `RuntimeStoreParts`, everything is already
    /// type-erased. The resolver's result writer isn't ready yet at this
    /// point either, so it's bound later via
    /// `AwaitEdgeSettler::bind_result_writer` (a deferred-binding trait
    /// method mirroring `bind_coordinator`).
    subagent_await_edge_writer: Arc<dyn AwaitEdgeWriter>,
    subagent_await_edge_settler: Arc<dyn AwaitEdgeSettler>,
    subagent_await_edge_evidence: Arc<dyn AwaitDependentRunEvidenceStore>,
    trigger_repository: Option<Arc<dyn ironclaw_triggers::TriggerRepository>>,
    /// Unified turn-run snapshot source for the substrate-agnostic trigger
    /// poller's active-run lookup. Local sets it from `local_runtime.turn_state`;
    /// production from `graph.turn_state`. Both back
    /// `SnapshotActiveRunLookup` identically, so the poller-launch block no
    /// longer reaches into the concrete local substrate for it.
    turn_run_snapshot_source: Arc<dyn TurnRunSnapshotSource>,
}

fn local_runtime_parts(
    local_runtime: &crate::factory::RebornRuntimeSubstrate,
) -> RuntimeStoreParts<'_> {
    let subagent_goal_store = Arc::new(FilesystemSubagentGoalStore::new(Arc::clone(
        &local_runtime.subagent_goal_filesystem,
    ))) as Arc<dyn RuntimeSubagentGoalStore>;

    let (subagent_await_edge_writer, subagent_await_edge_settler, subagent_await_edge_evidence) = {
        let store = Arc::new(FilesystemAwaitEdgeStore::new(Arc::clone(
            &local_runtime.subagent_goal_filesystem,
        )));
        let resolver = Arc::new(AwaitEdgeResolver::new_unbound_deferred_result_writer(
            Arc::clone(&store),
            Arc::clone(&subagent_goal_store) as Arc<dyn ironclaw_loop_host::SubagentSpawnGoalStore>,
            Arc::clone(&local_runtime.turn_state)
                as Arc<dyn ironclaw_turns::TurnSpawnTreeStateStore>,
            Arc::clone(&local_runtime.thread_service),
        ));
        let driver = Arc::new(ScopeRecoveryDriver::new(
            Arc::clone(&resolver),
            Arc::clone(&store),
        ));
        (
            driver as Arc<dyn AwaitEdgeWriter>,
            resolver as Arc<dyn AwaitEdgeSettler>,
            store as Arc<dyn AwaitDependentRunEvidenceStore>,
        )
    };

    RuntimeStoreParts {
        local_runtime: Some(local_runtime),
        turn_state_store: Arc::clone(&local_runtime.turn_state) as Arc<dyn RuntimeTurnStateStore>,
        checkpoint_state_store: Arc::clone(&local_runtime.checkpoint_state_store),
        loop_checkpoint_store: Arc::clone(&local_runtime.loop_checkpoint_store),
        thread_service: Arc::clone(&local_runtime.thread_service),
        event_log: Arc::clone(&local_runtime.event_log),
        audit_log: Arc::clone(&local_runtime.audit_log),
        resource_governor: Arc::clone(&local_runtime.resource_governor),
        budget_gate_store: Arc::clone(&local_runtime.budget_gate_store),
        broadcast_budget_event_sink: Arc::clone(&local_runtime.broadcast_budget_event_sink),
        subagent_goal_store,
        subagent_await_edge_writer,
        subagent_await_edge_settler,
        subagent_await_edge_evidence,
        trigger_repository: Some(Arc::clone(&local_runtime.trigger_repository)),
        turn_run_snapshot_source: Arc::clone(&local_runtime.turn_state)
            as Arc<dyn TurnRunSnapshotSource>,
    }
}

fn production_runtime_parts<F>(
    graph: &Arc<crate::factory::RebornProductionRuntimeStoreGraph<F>>,
) -> RuntimeStoreParts<'static>
where
    F: RootFilesystem + 'static,
{
    let subagent_goal_store = Arc::new(FilesystemSubagentGoalStore::new(Arc::clone(
        &graph.scoped_filesystem,
    ))) as Arc<dyn RuntimeSubagentGoalStore>;

    let await_edge_store = Arc::new(FilesystemAwaitEdgeStore::new(Arc::clone(
        &graph.scoped_filesystem,
    )));
    let await_edge_resolver = Arc::new(AwaitEdgeResolver::new_unbound_deferred_result_writer(
        Arc::clone(&await_edge_store),
        Arc::clone(&subagent_goal_store) as Arc<dyn ironclaw_loop_host::SubagentSpawnGoalStore>,
        Arc::clone(&graph.turn_state) as Arc<dyn ironclaw_turns::TurnSpawnTreeStateStore>,
        Arc::clone(&graph.thread_service),
    ));
    let await_edge_driver = Arc::new(ScopeRecoveryDriver::new(
        Arc::clone(&await_edge_resolver),
        Arc::clone(&await_edge_store),
    ));

    RuntimeStoreParts {
        local_runtime: None,
        turn_state_store: Arc::clone(&graph.turn_state) as Arc<dyn RuntimeTurnStateStore>,
        checkpoint_state_store: Arc::clone(&graph.checkpoint_state_store),
        loop_checkpoint_store: Arc::clone(&graph.turn_state)
            as Arc<dyn ironclaw_turns::LoopCheckpointStore>,
        thread_service: Arc::clone(&graph.thread_service),
        event_log: Arc::clone(&graph.event_log),
        audit_log: Arc::clone(&graph.audit_log),
        resource_governor: Arc::clone(&graph.resource_governor),
        budget_gate_store: Arc::clone(&graph.budget_gate_store),
        broadcast_budget_event_sink: Arc::clone(&graph.broadcast_budget_event_sink),
        subagent_goal_store,
        subagent_await_edge_writer: await_edge_driver as Arc<dyn AwaitEdgeWriter>,
        subagent_await_edge_settler: await_edge_resolver as Arc<dyn AwaitEdgeSettler>,
        subagent_await_edge_evidence: await_edge_store as Arc<dyn AwaitDependentRunEvidenceStore>,
        trigger_repository: Some(Arc::clone(&graph.trigger_repository)),
        turn_run_snapshot_source: Arc::clone(&graph.turn_state) as Arc<dyn TurnRunSnapshotSource>,
    }
}

/// Gate live-traffic startup on the deployment's [`TrafficPolicy`].
///
/// §4.4: this used to be a seven-arm `match` on the composition profile, with
/// each arm spelling out its own readiness precondition. The precondition is
/// now data on the config — a required readiness state plus an optional
/// production-blocking-diagnostic veto — so this reads one value. The profile
/// still appears in the error text, as a label for the operator.
fn enforce_runtime_cutover_gate(
    deployment: &DeploymentConfig,
    readiness: &RebornReadiness,
) -> Result<(), RebornRuntimeError> {
    let profile = deployment.profile();
    let traffic = deployment.traffic();
    if let Some(reason) = traffic.live_traffic_refusal(profile) {
        return Err(RebornRuntimeError::InvalidArgument { reason });
    }
    if let TrafficPolicy::Serve {
        required_readiness,
        veto_on_production_blocking_diagnostic,
    } = traffic
    {
        if readiness.state != required_readiness {
            return Err(RebornRuntimeError::InvalidArgument {
                reason: format!(
                    "profile={profile} cannot start Reborn runtime before readiness is validated; required_state={required_readiness:?}, state={:?}",
                    readiness.state
                ),
            });
        }
        if veto_on_production_blocking_diagnostic
            && let Some(diagnostic) = readiness
                .diagnostics
                .iter()
                .find(|diagnostic| diagnostic.blocks_production)
        {
            return Err(RebornRuntimeError::InvalidArgument {
                reason: format!(
                    "profile={profile} cannot start Reborn runtime while readiness diagnostic blocks production: component={:?}, reason={:?}",
                    diagnostic.component, diagnostic.reason
                ),
            });
        }
    }
    Ok(())
}

/// Guard: production and migration-dry-run compositions always pre-mint
/// [`SchedulerWakeWiring`] in `build_production_shaped` so the
/// `HostRuntimeServices` notifier and the scheduler wake loop share exactly one
/// channel. If the wiring is `None` for those profiles it means the composition
/// contract was violated (e.g. a code path forgot to mint it), and starting the
/// runtime would silently create a divergent scheduler-local channel. Extracted
/// so the negative branch is unit-testable without a full libsql/postgres
/// substrate.
fn check_production_scheduler_wake_wiring(
    profile: RebornCompositionProfile,
    wiring: &Option<ironclaw_runner::runtime::SchedulerWakeWiring>,
) -> Result<(), RebornRuntimeError> {
    if wiring.is_none()
        && DeploymentConfig::for_profile(profile, false).substrate()
            == RuntimeSubstrate::ProductionShaped
    {
        return Err(RebornRuntimeError::InvalidArgument {
            reason: "production runtime missing scheduler wake wiring".to_string(),
        });
    }
    Ok(())
}

mod approval;
mod auth_interaction;
#[cfg(test)]
#[path = "runtime/tests/auth_interaction.rs"]
mod auth_interaction_tests;
#[cfg(test)]
#[path = "runtime/tests/default_system_prompt.rs"]
mod default_system_prompt_tests;
mod latency;
mod local_dev;
#[cfg(test)]
#[path = "runtime/tests/outbound_delivery.rs"]
mod outbound_delivery_tests;
mod production;
mod runtime_turn_scheduler;
mod skills;
#[cfg(feature = "test-support")]
#[path = "runtime/test_support.rs"]
mod test_support;

#[cfg(feature = "test-support")]
pub(crate) use local_dev::PROJECT_CREATE_CAPABILITY_ID;
#[cfg(feature = "test-support")]
pub(crate) use local_dev::RESULT_READ_CAPABILITY_ID_FOR_TEST;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use local_dev::SKILL_ACTIVATE_CAPABILITY_ID;

pub use skills::{
    RebornSkillActivation, RebornSkillActivationMode, RebornSkillAsset, RebornSkillBundle,
    RebornSkillExecutionPlan, RebornSkillExecutionResult, RebornSkillSourceKind,
};

use skills::skill_asset_error;

use crate::runtime_input::ResolvedRebornLlm;

/// Stable identifier for a Reborn CLI conversation. Wraps a `ThreadId`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConversationId(pub ThreadId);

/// Final-form assistant reply read back from the session thread service after
/// a `send_user_message` completes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantReply {
    pub conversation: ConversationId,
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub failure_category: Option<String>,
    pub text: Option<String>,
}

impl AssistantReply {
    /// True when a caller can treat the reply as a successful single-shot
    /// response. Recovery/failed/cancelled runs may still produce diagnostics,
    /// but they did not produce the requested assistant text.
    pub fn is_successful_final_reply(&self) -> bool {
        self.status == TurnStatus::Completed && self.text.is_some()
    }
}

/// Accepted-turn handle returned by `RebornRuntime::submit_user_turn`. Holds
/// the per-conversation send lock for its lifetime so the caller's wait phase
/// retains the same mutual exclusion the inline submit path used to.
struct SubmittedTurn {
    _send_guard: OwnedMutexGuard<()>,
    scope: TurnScope,
    run_id: TurnRunId,
    accepted_message_ref: AcceptedMessageRef,
}

/// Outcome of driving a single turn that may pause on a gate.
///
/// Test/recording-support only — produced by
/// [`RebornRuntime::send_user_message_until_gate`], which mirrors the
/// production [`RebornRuntime::send_user_message`] submit path but returns when
/// the run first reaches a terminal status *or* parks on a `Blocked*` gate,
/// instead of waiting only for a terminal status. Gate *resolution* stays on
/// the WebUI `ProductSurface` facade (`resolve_gate`) per the #3094 seam;
/// this type only observes where a run paused.
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone)]
pub enum RebornTurnDriveOutcome {
    /// The run reached a terminal status without pausing on a gate.
    Terminal(AssistantReply),
    /// The run parked on a user-resolvable gate (auth/approval/resource) and is
    /// awaiting resolution through the facade. `gate_ref` is required: the
    /// blocked-reason contract carries a `GateRef` for every such block, so its
    /// absence is an invariant violation, not a valid recorder outcome.
    BlockedOnGate {
        run_id: TurnRunId,
        status: TurnStatus,
        gate_ref: ironclaw_turns::GateRef,
        partial_text: Option<String>,
    },
}

/// Errors returned by `RebornRuntime` methods.
#[derive(Debug, Error)]
pub enum RebornRuntimeError {
    #[error("reborn runtime build failed: {0}")]
    Build(#[from] RebornBuildError),
    #[error("turn coordinator unavailable for assembled runtime")]
    TurnCoordinatorUnavailable,
    #[error("host runtime unavailable for assembled runtime")]
    HostRuntimeUnavailable,
    #[error("turn submission failed: {0}")]
    TurnSubmission(String),
    #[error("turn submission rejected: {reason}")]
    TurnRejected { reason: String },
    #[error("session thread service error: {0}")]
    ThreadService(String),
    #[error("turn coordinator error: {0}")]
    TurnCoordinator(String),
    #[error("run did not reach a terminal state within {timeout:?}")]
    RunTimeout { timeout: Duration },
    #[error("run cancelled by caller")]
    OperationCancelled,
    #[error("invalid scope or identifier: {reason}")]
    InvalidArgument { reason: String },
    #[error("malformed runtime configuration: {reason}")]
    MalformedConfig { reason: String },
    #[error("llm provider construction failed: {0}")]
    LlmProvider(String),
    #[error("turn-runner worker is no longer running")]
    WorkerStopped,
    #[error("skill execution unavailable for assembled runtime")]
    SkillExecutionUnavailable,
    #[error("skill execution failed: {0}")]
    SkillExecution(String),
}

impl From<TurnError> for RebornRuntimeError {
    fn from(value: TurnError) -> Self {
        Self::TurnCoordinator(value.to_string())
    }
}

impl From<DefaultPlannedRuntimeBuildError> for RebornRuntimeError {
    fn from(value: DefaultPlannedRuntimeBuildError) -> Self {
        Self::InvalidArgument {
            reason: value.to_string(),
        }
    }
}

/// Started, running Reborn agent runtime.
///
/// `RebornRuntime` is the single user-facing handle returned by
/// [`build_reborn_runtime`]. Downstream code never reaches into the substrate
/// or worker machinery: it talks to the runtime through task-level methods.
pub struct RebornRuntime {
    services: RebornServices,
    turn_coordinator: Arc<dyn TurnCoordinator>,
    /// Generic channel host assembly (extension-runtime P6 S2): the
    /// per-extension inbound-channel reconcile loop over the generic host's
    /// active snapshot. `None` when the composition path has no generic
    /// host. Never read after construction — held purely so the reconcile
    /// loop lives exactly as long as the runtime (dropping the `Arc` ends
    /// the loop).
    #[allow(
        dead_code,
        reason = "owned so the reconcile loop lives with the runtime"
    )]
    channel_host_assembly:
        Option<Arc<crate::extension_host::channel_host::GenericChannelHostAssembly>>,
    /// Turn-state row store, kept so graceful `shutdown` can drain the
    /// write-behind durable tail (awaiting the acks of non-critical transitions
    /// that committed at memory speed) so a planned restart recovers in-flight
    /// turns, not just the synchronously-durable gate-park/terminal/new-run ones.
    /// `None` when no local runtime is wired (e.g. production-parts launches).
    turn_state_flush: Option<Arc<ComposedTurnStateStore>>,
    turn_tree_store: Arc<dyn TurnSpawnTreeStateStore>,
    thread_service: Arc<dyn SessionThreadService>,
    thread_scope: ThreadScope,
    turn_scheduler: RuntimeTurnScheduler,
    trigger_poller_handle: Option<TriggerPollerRuntimeHandle>,
    credential_refresh_worker_handle: Option<ironclaw_auth::KeepaliveSweepHandle>,
    trace_flush_worker: crate::observability::trace_capture::TraceQueueFlushWorkerHandle,
    skill_learning_extraction_tasks:
        Option<Arc<crate::extension_host::skill_learning::SkillLearningExtractionTasks>>,
    #[cfg(any(test, feature = "test-support"))]
    trigger_conversation_pairing:
        Option<Arc<dyn ironclaw_conversations::ConversationActorPairingService>>,
    outbound_delivery_target_registry: Option<Arc<MutableOutboundDeliveryTargetRegistry>>,
    budget_event_projection: Option<crate::observability::budget_events::BudgetEventProjection>,
    poll_settings: PollSettings,
    /// Mints the one-time API bearer on admin user creation. Read by
    /// `build_webui_services` when wiring the admin surface. `None` leaves the
    /// admin create path reporting the token minter unavailable.
    admin_api_token_minter: Option<Arc<dyn crate::AdminApiTokenMinter>>,
    actor_user_id: UserId,
    source_binding_ref: SourceBindingRef,
    reply_target_binding_ref: ReplyTargetBindingRef,
    projection_services: RebornProjectionServices,
    approval_interaction_service: Arc<dyn ApprovalInteractionService>,
    auth_interaction_service: Arc<dyn AuthInteractionService>,
    #[cfg(test)]
    approval_audit_sink: Arc<InMemoryAuditSink>,
    webui_event_log: Arc<dyn DurableEventLog>,
    default_run_profile_id: String,
    send_locks: Mutex<HashMap<ConversationId, Arc<Mutex<()>>>>,
    skill_activation_source: Option<Arc<ComposedSelectableSkillContextSource>>,
    skill_execution_adapter: Option<Arc<ComposedSkillExecutionAdapter>>,
    /// Operator boot config, carried so the WebUI facade can compose the
    /// LLM-config settings service over `providers.json` / `config.toml`.
    boot: Option<ironclaw_reborn_config::RebornBootConfig>,
    /// Hot-swap handle for the live LLM provider, when one was wired at boot.
    llm_reload: Option<RebornLlmReloadParts>,
}

struct RegistryPersistentApprovalGranteeResolver {
    registry: Arc<ExtensionRegistry>,
    outbound_delivery_target_set_provider: ExtensionId,
}

impl PersistentApprovalGranteeResolver for RegistryPersistentApprovalGranteeResolver {
    fn persistent_approval_grantee(&self, capability_id: &CapabilityId) -> Option<Principal> {
        if let Some(descriptor) = self.registry.get_capability(capability_id) {
            return Some(Principal::Extension(descriptor.provider.clone()));
        }
        if capability_id.as_str() == OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID {
            return Some(Principal::Extension(
                self.outbound_delivery_target_set_provider.clone(),
            ));
        }
        None
    }
}

impl RegistryPersistentApprovalGranteeResolver {
    fn new(registry: Arc<ExtensionRegistry>) -> Result<Self, RebornRuntimeError> {
        let outbound_delivery_target_set_provider = outbound_delivery_synthetic_provider()
            .map_err(|error| RebornRuntimeError::InvalidArgument {
                reason: format!("outbound delivery synthetic provider id is invalid: {error}"),
            })?;
        Ok(Self {
            registry,
            outbound_delivery_target_set_provider,
        })
    }
}

/// Shared local-dev `DefaultApprovalInteractionService` wiring recipe. Used by both
/// `build_reborn_runtime` and `test_support::local_dev_approval_interaction_service_for_test`
/// so the two never drift (W5-WEBUI-API-2 follow-up). `audit_sink` is `None` from the
/// test accessor: production wires one for audit-log observability only, not
/// correctness the test needs. Propagates policy/resolver construction failures
/// instead of collapsing them to `None`. Thin wrapper over
/// `build_approval_interaction_service_with_turn_run_source` using
/// `local_runtime.turn_state` as the turn-run snapshot source — production
/// behavior is unchanged by the seam below.
pub(crate) fn build_approval_interaction_service(
    local_runtime: &crate::factory::RebornRuntimeSubstrate,
    builtin_capability_policy: Arc<BuiltinCapabilityPolicy>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
    audit_sink: Option<Arc<dyn ironclaw_events::AuditSink>>,
) -> Result<Arc<dyn ApprovalInteractionService>, RebornRuntimeError> {
    build_approval_interaction_service_with_turn_run_source(
        local_runtime,
        builtin_capability_policy,
        turn_coordinator,
        audit_sink,
        Arc::clone(&local_runtime.turn_state) as Arc<dyn TurnRunSnapshotSource>,
    )
}

/// Identical to [`build_approval_interaction_service`]
/// except the approval turn-run locator reads `turn_run_source` instead of
/// always deriving it from `local_runtime.turn_state`. Lets a caller whose
/// real runs live in a DIFFERENT `TurnStateStore` composition (e.g.
/// `RebornIntegrationGroup`'s own `build_default_planned_runtime`, whose runs
/// are invisible to this crate's `local_runtime.turn_state`) substitute its
/// own store. `build_approval_interaction_service` is the
/// production entry point and is a thin wrapper over this function with
/// `local_runtime.turn_state` as the source, so production behavior is
/// unchanged.
pub(crate) fn build_approval_interaction_service_with_turn_run_source(
    local_runtime: &crate::factory::RebornRuntimeSubstrate,
    builtin_capability_policy: Arc<BuiltinCapabilityPolicy>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
    audit_sink: Option<Arc<dyn ironclaw_events::AuditSink>>,
    turn_run_source: Arc<dyn TurnRunSnapshotSource>,
) -> Result<Arc<dyn ApprovalInteractionService>, RebornRuntimeError> {
    let approval_turn_runs = Arc::new(SnapshotApprovalTurnRunLocator::new(turn_run_source));
    let approval_read_model = Arc::new(RunStateApprovalInteractionReadModel::new(
        local_runtime.approval_requests.clone(),
        approval_turn_runs,
    ));
    let mut approval_resolver = ApprovalResolverPort::new(
        local_runtime.approval_requests.clone(),
        local_runtime.capability_leases.clone(),
    );
    if let Some(audit_sink) = audit_sink {
        approval_resolver = approval_resolver.with_audit_sink(audit_sink);
    }
    let approval_resolver = Arc::new(approval_resolver);

    Ok(Arc::new(
        DefaultApprovalInteractionService::new(
            approval_read_model,
            Arc::new(approval::PolicyApprovalLeaseTermsProvider::new(
                builtin_capability_policy,
                Arc::clone(&local_runtime.extension_registry),
                local_runtime.workspace_mounts.clone(),
                local_runtime.skill_mounts.clone(),
                local_runtime.memory_mounts.clone(),
                local_runtime.system_extensions_lifecycle_mounts.clone(),
                local_dev::extension_surface::ExtensionCapabilitySurfaceSource::new(
                    local_runtime.extension_management.clone(),
                ),
            )),
            approval_resolver,
            turn_coordinator,
        )
        .with_persistent_policy_store(local_runtime.persistent_approval_policies.clone())
        .with_persistent_grantee_resolver(Arc::new(RegistryPersistentApprovalGranteeResolver::new(
            Arc::clone(&local_runtime.extension_registry),
        )?))
        .with_tool_permission_override_store(local_runtime.tool_permission_overrides.clone()),
    ))
}

pub(crate) type ComposedSelectableSkillContextSource =
    SelectableSkillContextSource<FilesystemSkillBundleSource<CompositeRootFilesystem>>;
type ComposedSkillExecutionAdapter =
    SkillExecutionAdapter<FilesystemSkillBundleSource<CompositeRootFilesystem>>;

// TODO(#4416): when a second test-only handle is
// needed off the trigger poller seam (e.g. trusted_submitter,
// materializer, active_run_lookup for cleanup-state tests), consolidate
// the cfg-gated fields into a dedicated `TriggerPollerTestHandles`
// struct exposed via a single `RebornRuntime::trigger_poller_test_handles()`
// accessor. That removes the current `TriggerPollerServices` /
// `TriggerPollerServicesInner` split (review f-ptr-1/f-ptr-2) without
// inventing cfg-gated function parameters. Premature today: only one
// test-only handle exists, so the shape isn't proven yet.
struct TriggerPollerServices {
    materializer: Arc<dyn ironclaw_triggers::TriggerPromptMaterializer>,
    trusted_submitter: Arc<dyn ironclaw_triggers::TrustedTriggerFireSubmitter>,
    /// Late-binding slot for the post-submit hook. Created here and shared
    /// with the poller wrapper; filled by the composition root (the generic
    /// triggered-delivery hook) without restarting the poller.
    post_submit_hook_slot: Arc<
        std::sync::OnceLock<Arc<dyn crate::automation::trigger_poller::PostSubmitDeliveryHook>>,
    >,
    /// Test-support handle on the SAME conversation services instance the
    /// poller-side materializer/submitter use, so integration tests can call
    /// the production `pair_external_actor` API to seed the trigger
    /// creator's actor pairing before driving the poller. Without this
    /// pre-seed, real `ConversationContentRefMaterializer` fails closed with
    /// `BindingRequired` — by design — and the trusted-ingress turn is
    /// never submitted.
    #[cfg(any(test, feature = "test-support"))]
    pairing_service: Arc<dyn ironclaw_conversations::ConversationActorPairingService>,
}

/// Build the trigger-poller services from already-resolved conversation
/// services. Substrate-agnostic: the caller resolves the concrete conversation
/// services type per substrate (local `RebornFilesystemConversationServices` /
/// `InMemoryConversationServices`, or the production graph's
/// `RebornFilesystemConversationServices`) and hands it in by value, so this
/// function no longer reaches into the local substrate or branches on the
/// durable-backend cfg.
fn build_trigger_poller_services<C>(
    conversation_services: C,
    turn_coordinator: Arc<dyn TurnCoordinator>,
    thread_service: Arc<dyn SessionThreadService>,
    authorizer_config: TriggerPollerAuthorizerConfig,
    access_checker: Option<Arc<dyn crate::runtime_input::TriggerFireAccessChecker>>,
    tenant_id: TenantId,
    default_agent_id: AgentId,
) -> Result<TriggerPollerServices, RebornRuntimeError>
where
    C: ironclaw_conversations::ConversationBindingService
        + ironclaw_conversations::SessionThreadService
        + ironclaw_conversations::ConversationActorPairingService
        + Clone
        + 'static,
{
    let authorizer = build_trigger_fire_authorizer(authorizer_config, access_checker, tenant_id)?;
    #[cfg(any(test, feature = "test-support"))]
    let pairing_service: Arc<dyn ironclaw_conversations::ConversationActorPairingService> =
        Arc::new(conversation_services.clone());
    let TriggerPollerServicesInner {
        materializer,
        trusted_submitter,
    } = build_trigger_poller_services_from_conversation_services(
        conversation_services.clone(),
        conversation_services,
        turn_coordinator,
        thread_service,
        default_agent_id,
        authorizer,
    );
    Ok(TriggerPollerServices {
        materializer,
        trusted_submitter,
        post_submit_hook_slot: Arc::new(std::sync::OnceLock::new()),
        #[cfg(any(test, feature = "test-support"))]
        pairing_service,
    })
}

fn trigger_poller_authorization_required_error() -> RebornRuntimeError {
    RebornRuntimeError::InvalidArgument {
        reason: "trigger poller cannot be enabled without a fire-time creator access checker"
            .to_string(),
    }
}

/// Validate the temporary trigger-poller authorizer shape after the caller has
/// already decided to enable the poller.
fn validate_trigger_poller_authorization(
    trigger_poller: &TriggerPollerSettings,
    access_checker: Option<&Arc<dyn crate::runtime_input::TriggerFireAccessChecker>>,
) -> Result<(), RebornRuntimeError> {
    debug_assert!(trigger_poller.enabled);
    match trigger_poller.authorizer {
        #[cfg(any(test, feature = "test-support"))]
        TriggerPollerAuthorizerConfig::TenantScopedPlaceholderForTest => Ok(()),
        TriggerPollerAuthorizerConfig::CreatorAccessRequired => access_checker
            .map(|_| ())
            .ok_or_else(trigger_poller_authorization_required_error),
    }
}

fn build_trigger_fire_authorizer(
    authorizer_config: TriggerPollerAuthorizerConfig,
    access_checker: Option<Arc<dyn crate::runtime_input::TriggerFireAccessChecker>>,
    tenant_id: TenantId,
) -> Result<
    Arc<dyn crate::automation::trigger_poller_trusted_submit::TriggerFireAuthorizer>,
    RebornRuntimeError,
> {
    #[cfg(not(any(test, feature = "test-support")))]
    let _ = tenant_id;
    match authorizer_config {
        #[cfg(any(test, feature = "test-support"))]
        TriggerPollerAuthorizerConfig::TenantScopedPlaceholderForTest => Ok(Arc::new(
            TenantScopedTrustedTriggerFireAuthorizer::new(tenant_id),
        )),
        TriggerPollerAuthorizerConfig::CreatorAccessRequired => access_checker
            .map(|checker| {
                Arc::new(AccessCheckerTriggerFireAuthorizer::new(checker))
                    as Arc<
                        dyn crate::automation::trigger_poller_trusted_submit::TriggerFireAuthorizer,
                    >
            })
            .ok_or_else(trigger_poller_authorization_required_error),
    }
}

struct TriggerPollerServicesInner {
    materializer: Arc<dyn ironclaw_triggers::TriggerPromptMaterializer>,
    trusted_submitter: Arc<dyn ironclaw_triggers::TrustedTriggerFireSubmitter>,
}

fn build_trigger_poller_services_from_conversation_services<B, S>(
    binding_service: B,
    session_thread_service: S,
    turn_coordinator: Arc<dyn TurnCoordinator>,
    thread_service: Arc<dyn SessionThreadService>,
    default_agent_id: AgentId,
    authorizer: Arc<dyn crate::automation::trigger_poller_trusted_submit::TriggerFireAuthorizer>,
) -> TriggerPollerServicesInner
where
    B: ironclaw_conversations::ConversationBindingService + Clone + 'static,
    S: ironclaw_conversations::SessionThreadService + 'static,
{
    let materializer = Arc::new(ConversationContentRefMaterializer::new(
        binding_service.clone(),
        Arc::clone(&thread_service),
        default_agent_id.clone(),
        authorizer,
    ));
    let trusted_submitter = ironclaw_conversations::trusted_trigger_fire_submitter(
        binding_service,
        session_thread_service,
        turn_coordinator,
    );
    TriggerPollerServicesInner {
        materializer,
        trusted_submitter,
    }
}

fn build_trigger_active_run_lookup(
    snapshot_source: Arc<dyn TurnRunSnapshotSource>,
) -> Arc<dyn ironclaw_triggers::TriggerActiveRunLookup> {
    Arc::new(SnapshotActiveRunLookup::new(snapshot_source))
}

/// Resolve the fire-time `TenantMembership` user directory from either
/// substrate. Mirrors [`RebornRuntime::reborn_user_directory`] but as a free
/// function usable during `build_reborn_runtime` (before a `RebornRuntime`
/// exists): local builds it over the local substrate's identity filesystem;
/// production-shaped builds match the production store graph and build it over
/// the same scoped filesystem the identity resolver uses. Returns `None` only
/// when no substrate is present, in which case a `TenantMembership` grant
/// cannot be honored and the caller fails closed.
fn poller_user_directory(
    services: &RebornServices,
    local_runtime: Option<&crate::factory::RebornRuntimeSubstrate>,
    tenant_id: &TenantId,
    actor_user_id: &UserId,
    agent_id: &AgentId,
    project_id: Option<&ProjectId>,
) -> Option<Arc<dyn ironclaw_reborn_identity::RebornUserDirectory>> {
    if let Some(local) = local_runtime {
        let store = ironclaw_reborn_identity::FilesystemRebornIdentityStore::new(
            Arc::clone(&local.identity_filesystem),
            tenant_id.clone(),
            actor_user_id.clone(),
            agent_id.clone(),
            project_id.cloned(),
        );
        return Some(Arc::new(store) as Arc<dyn ironclaw_reborn_identity::RebornUserDirectory>);
    }
    if let Some(production) = services.production_runtime.as_ref() {
        let directory: Arc<dyn ironclaw_reborn_identity::RebornUserDirectory> = match production {
            crate::factory::RebornProductionRuntimeServices::LibSql(graph) => Arc::new(
                ironclaw_reborn_identity::FilesystemRebornIdentityStore::new(
                    Arc::clone(&graph.scoped_filesystem),
                    tenant_id.clone(),
                    actor_user_id.clone(),
                    agent_id.clone(),
                    project_id.cloned(),
                ),
            ),
            crate::factory::RebornProductionRuntimeServices::Postgres(graph) => Arc::new(
                ironclaw_reborn_identity::FilesystemRebornIdentityStore::new(
                    Arc::clone(&graph.scoped_filesystem),
                    tenant_id.clone(),
                    actor_user_id.clone(),
                    agent_id.clone(),
                    project_id.cloned(),
                ),
            ),
        };
        return Some(directory);
    }
    None
}

struct SnapshotApprovalTurnRunLocator {
    /// A trait object (not the concrete `ComposedTurnStateStore`) so a
    /// caller can substitute a different turn-state store's snapshot view —
    /// see `turn_run_snapshot::TurnRunSnapshotSource` and
    /// `build_approval_interaction_service_with_turn_run_source`.
    turn_state: Arc<dyn TurnRunSnapshotSource>,
}

impl SnapshotApprovalTurnRunLocator {
    fn new(turn_state: Arc<dyn TurnRunSnapshotSource>) -> Self {
        Self { turn_state }
    }

    async fn snapshot(
        &self,
    ) -> Result<TurnPersistenceSnapshot, ironclaw_product_workflow::ProductWorkflowError> {
        self.turn_state.turn_run_snapshot().await.map_err(|error| {
            tracing::debug!(
                %error,
                "approval turn-run locator could not read turn persistence snapshot"
            );
            approval_turn_locator_unavailable()
        })
    }
}

struct ApprovalRequestGateEvidence {
    approval_requests: Arc<dyn ironclaw_run_state::ApprovalRequestStore>,
}

/// Test-only constructor for [`ApprovalRequestGateEvidence`].
///
/// Mirrors the production wiring in `build_local_runtime` (runtime.rs ~line 2799)
/// where `ApprovalRequestGateEvidence` is constructed inline and passed to
/// `loop_exit_evidence.with_approval_gate_evidence`. Exists so `test_support.rs`
/// can build the real evidence type without needing the struct or its field to be
/// `pub(crate)`. For tests only — gated behind `test-support`, ships zero bytes
/// in production binaries.
#[cfg(feature = "test-support")]
pub(crate) fn build_approval_gate_evidence_for_test(
    approval_requests: std::sync::Arc<dyn ironclaw_run_state::ApprovalRequestStore>,
) -> std::sync::Arc<dyn ironclaw_runner::loop_exit_applier::ApprovalGateEvidenceStore> {
    std::sync::Arc::new(ApprovalRequestGateEvidence { approval_requests })
}

/// Test-support forwarder for the `result_read` synthetic-capability wrap
/// (durable tool-result projection seam, issue #5838). Bridges the private
/// `local_dev` module to `test_support.rs`; mirrors the `project_create`
/// forwarder above.
#[cfg(feature = "test-support")]
pub(crate) fn wrap_result_read_capability_for_test(
    inner: std::sync::Arc<dyn ironclaw_turns::run_profile::LoopCapabilityPort>,
    thread_service: std::sync::Arc<dyn ironclaw_threads::SessionThreadService>,
    fallback_user_id: ironclaw_host_api::UserId,
    run_context: ironclaw_turns::run_profile::LoopRunContext,
    input_resolver: std::sync::Arc<dyn ironclaw_loop_host::LoopCapabilityInputResolver>,
    result_writer: std::sync::Arc<dyn ironclaw_loop_host::LoopCapabilityResultWriter>,
) -> Result<
    std::sync::Arc<dyn ironclaw_turns::run_profile::LoopCapabilityPort>,
    ironclaw_turns::run_profile::AgentLoopHostError,
> {
    local_dev::wrap_result_read_capability_for_test(
        inner,
        thread_service,
        fallback_user_id,
        run_context,
        input_resolver,
        result_writer,
    )
}

/// Test-support forwarder (E-SKILL seam): build the local-dev filesystem skill
/// context source exactly as production does in [`build_reborn_runtime`], and
/// hand back just the `HostSkillContextSource` (for prompt injection) plus the
/// `activation_source` (backing `skill_activate`) that `test_support.rs` needs.
/// Reuses the private [`local_dev_filesystem_skill_context_source`] so the
/// wiring never drifts, but deliberately does NOT return the internal
/// [`ComposedSkillContextSource`] struct — that type (and its
/// `execution_adapter` field) stays private to this module; only the two
/// fields an external caller actually consumes cross the boundary. Tests only.
#[cfg(feature = "test-support")]
pub(crate) fn local_dev_filesystem_skill_context_source_for_test(
    local_runtime: &crate::factory::RebornRuntimeSubstrate,
    tenant_id: &TenantId,
    regex_skill_activation_enabled: bool,
) -> Result<
    (
        Arc<dyn HostSkillContextSource>,
        Arc<ComposedSelectableSkillContextSource>,
    ),
    RebornRuntimeError,
> {
    let built = local_dev_filesystem_skill_context_source(
        local_runtime,
        tenant_id,
        regex_skill_activation_enabled,
    )?;
    Ok((built.source, built.activation_source))
}

/// Test-support forwarder (harness-port-seam P1 seam) for
/// `create_refreshing_capability_port`
/// (`refreshing_capability_port.rs:75`), production's sole capability-port
/// factory. Bridges the private `local_dev` module to `test_support`; mirrors
/// the `outbound_delivery` forwarder above. For tests only -- gated behind
/// `test-support`, ships zero bytes in production builds.
#[cfg(feature = "test-support")]
pub(crate) async fn create_refreshing_capability_port_for_test(
    parts: crate::test_support::RefreshingCapabilityPortTestParts,
) -> Result<
    std::sync::Arc<dyn ironclaw_turns::run_profile::LoopCapabilityPort>,
    ironclaw_turns::run_profile::AgentLoopHostError,
> {
    local_dev::create_refreshing_capability_port_for_test(parts).await
}

/// Test-support forwarder exposing production's real `StagedCapabilityIo`
/// wiring (`local_dev.rs`'s `staged_capability_io_for_test`, which mirrors
/// `capability_wiring`'s `new_with_durable_previews` call). Bridges the
/// private `local_dev` module to `test_support`; mirrors the
/// `create_refreshing_capability_port_for_test` forwarder above.
/// For tests only -- gated behind `test-support`, ships zero bytes in
/// production builds.
#[cfg(feature = "test-support")]
pub(crate) fn staged_capability_io_for_test(
    thread_service: std::sync::Arc<dyn ironclaw_threads::SessionThreadService>,
    fallback_user_id: ironclaw_host_api::UserId,
) -> (
    std::sync::Arc<dyn ironclaw_loop_host::LoopCapabilityInputResolver>,
    std::sync::Arc<dyn ironclaw_loop_host::LoopCapabilityResultWriter>,
) {
    local_dev::staged_capability_io_for_test(thread_service, fallback_user_id)
}

#[async_trait::async_trait]
impl ApprovalGateEvidenceStore for ApprovalRequestGateEvidence {
    async fn pending_approval_gate(
        &self,
        scope: &TurnScope,
        gate_ref: &LoopGateRef,
    ) -> Result<bool, TurnError> {
        let Some(request_id) = approval_request_id_from_gate_ref(gate_ref) else {
            return Ok(false);
        };
        let record = self
            .approval_requests
            .get(&scope.to_resource_scope(), request_id)
            .await
            .map_err(|error| TurnError::Unavailable {
                reason: format!("approval request evidence lookup failed: {error}"),
            })?;
        Ok(record
            .map(|record| record.status == ironclaw_run_state::ApprovalStatus::Pending)
            .unwrap_or(false))
    }
}

fn approval_request_id_from_gate_ref(gate_ref: &LoopGateRef) -> Option<ApprovalRequestId> {
    gate_ref
        .as_str()
        .strip_prefix("gate:approval-")
        .and_then(|value| ApprovalRequestId::parse(value).ok())
}

#[async_trait::async_trait]
impl ApprovalTurnRunLocator for SnapshotApprovalTurnRunLocator {
    async fn blocked_approval_runs(
        &self,
        scope: &ApprovalInteractionScope,
    ) -> Result<Vec<ApprovalBlockedTurnRun>, ironclaw_product_workflow::ProductWorkflowError> {
        let turn_scope = TurnScope::new(
            scope.tenant_id.clone(),
            scope.agent_id.clone(),
            scope.project_id.clone(),
            scope.thread_id.clone(),
        );
        let actor = TurnActor::new(scope.user_id.clone());
        let snapshot = self.snapshot().await?;
        let mut runs = snapshot
            .runs
            .iter()
            .filter(|run| {
                run.scope.same_thread(&turn_scope)
                    && run.status == TurnStatus::BlockedApproval
                    && run.gate_ref.is_some()
                    && snapshot_run_actor_matches(&snapshot, run, &actor)
            })
            .filter_map(|run| {
                run.gate_ref.clone().map(|gate_ref| ApprovalBlockedTurnRun {
                    run_id: run.run_id,
                    gate_ref,
                })
            })
            .collect::<Vec<_>>();
        runs.sort_by_key(|run| run.run_id.as_uuid());
        Ok(runs)
    }

    async fn approval_run_for_gate(
        &self,
        scope: &ApprovalInteractionScope,
        gate_ref: &ironclaw_turns::GateRef,
    ) -> Result<Option<TurnRunId>, ironclaw_product_workflow::ProductWorkflowError> {
        let turn_scope = TurnScope::new(
            scope.tenant_id.clone(),
            scope.agent_id.clone(),
            scope.project_id.clone(),
            scope.thread_id.clone(),
        );
        let actor = TurnActor::new(scope.user_id.clone());
        let snapshot = self.snapshot().await?;
        let active = snapshot
            .runs
            .iter()
            .find(|run| {
                run.scope.same_thread(&turn_scope)
                    && run.status == TurnStatus::BlockedApproval
                    && run.gate_ref.as_ref() == Some(gate_ref)
                    && snapshot_run_actor_matches(&snapshot, run, &actor)
            })
            .map(|run| run.run_id);
        if active.is_some() {
            return Ok(active);
        }

        let mut historical = snapshot
            .checkpoints
            .iter()
            .filter(|checkpoint| {
                checkpoint.status == TurnStatus::BlockedApproval
                    && &checkpoint.gate_ref == gate_ref
                    && checkpoint
                        .scope
                        .as_ref()
                        .is_none_or(|stored| stored.same_thread(&turn_scope))
            })
            .filter_map(|checkpoint| {
                snapshot
                    .runs
                    .iter()
                    .find(|run| {
                        run.run_id == checkpoint.run_id
                            && run.scope.same_thread(&turn_scope)
                            && snapshot_run_actor_matches(&snapshot, run, &actor)
                    })
                    .map(|run| run.run_id)
            })
            .collect::<Vec<_>>();
        historical.sort_by_key(|run_id| run_id.as_uuid());
        historical.dedup();
        Ok(historical.into_iter().next())
    }
}

fn snapshot_run_actor_matches(
    snapshot: &TurnPersistenceSnapshot,
    run: &TurnRunRecord,
    actor: &TurnActor,
) -> bool {
    snapshot.turns.iter().any(|turn| {
        turn.turn_id == run.turn_id && turn.scope.same_thread(&run.scope) && turn.actor == *actor
    })
}

fn approval_turn_locator_unavailable() -> ironclaw_product_workflow::ProductWorkflowError {
    ironclaw_product_workflow::ProductWorkflowError::Transient {
        reason: "approval turn-run locator unavailable".to_string(),
    }
}

impl RebornRuntime {
    /// Snapshot of the substrate facades produced by `build_reborn_services`.
    /// Exposed for diagnostics / readiness reporting; **not** for traffic.
    pub fn services(&self) -> &RebornServices {
        &self.services
    }

    /// Seed a bare `secret_handle` secret for an owner scope so keyed
    /// capabilities (network + `use_secret`) can resolve their
    /// `InjectSecretOnce` obligation. `serve` uses this to write the value of
    /// an `IRONCLAW_REBORN_DEV_SECRET__<handle>` env var into the tenant-shared
    /// admin-managed scope, so one operator-provisioned key serves every user of
    /// the tenant (SSO users included) without per-user provisioning. The secret
    /// store is composition-private, so this is the single narrow write seam.
    pub async fn seed_local_dev_secret(
        &self,
        owner: ResourceScope,
        handle: ironclaw_host_api::SecretHandle,
        secret_value: String,
    ) -> Result<(), ironclaw_secrets::SecretStoreError> {
        self.services
            .secret_store()
            .put(
                owner,
                handle,
                ironclaw_secrets::SecretMaterial::from(secret_value),
                None,
            )
            .await
            .map(|_| ())
    }

    pub(crate) fn webui_tenant_id(&self) -> &TenantId {
        &self.thread_scope.tenant_id
    }

    #[cfg(test)]
    #[allow(
        dead_code,
        reason = "used only by selected test modules; feature-filtered all-target builds may not compile those call sites"
    )]
    pub(crate) fn clear_local_runtime_for_test(&mut self) {
        self.services.local_runtime = None;
        self.outbound_delivery_target_registry = None;
    }

    /// Operator boot config, when the runtime was assembled with one. The
    /// WebUI facade uses it to compose the LLM-config settings service.
    pub(crate) fn webui_boot_config(&self) -> Option<&ironclaw_reborn_config::RebornBootConfig> {
        self.boot.as_ref()
    }

    /// The runtime's NEAR AI session manager, when an LLM seam is wired. The
    /// LLM-config service uses it so a completed NEAR AI login applies to the
    /// live provider on reload.
    pub(crate) fn webui_llm_session(&self) -> Option<Arc<ironclaw_llm::SessionManager>> {
        self.llm_reload
            .as_ref()
            .map(|parts| Arc::clone(&parts.session))
    }

    /// Shared NEAR AI login-state store. The authenticated start endpoint
    /// issues states and the public callback consumes them.
    pub(crate) fn webui_nearai_login_states(
        &self,
    ) -> Option<Arc<crate::llm_admin::llm_config_service::NearAiLoginStateStore>> {
        self.llm_reload
            .as_ref()
            .map(|parts| Arc::clone(&parts.nearai_login_states))
    }

    /// Public NEAR AI login callback mount for the host ingress to merge via
    /// `ironclaw_webui::WebuiServeConfig::with_public_route_mount`. Built
    /// from the runtime's private session/reload/boot so those stay internal.
    /// `None` when no LLM seam or boot config was wired.
    pub fn nearai_login_callback_mount(
        &self,
    ) -> Option<crate::webui::route_mounts::PublicRouteMount> {
        let boot = self.boot.clone()?;
        let session = self.webui_llm_session()?;
        let reload = self.webui_llm_reload_trigger()?;
        let states = self.webui_nearai_login_states()?;
        Some(
            crate::llm_admin::nearai_login_serve::nearai_login_callback_mount(
                session, reload, boot, states,
            ),
        )
    }

    /// Live LLM-provider reload trigger for the settings service. Returns the
    /// hot-swap adapter when an LLM provider was wired at boot; otherwise
    /// `None`, in which case config edits persist to disk and apply on the
    /// next restart.
    pub(crate) fn webui_llm_reload_trigger(&self) -> Option<Arc<dyn crate::LlmReloadTrigger>> {
        let boot = self.boot.as_ref()?;
        let parts = self.llm_reload.as_ref()?;
        Some(Arc::new(
            crate::llm_admin::llm_reload::RebornLlmReloadAdapter::new(
                boot.clone(),
                Arc::clone(&parts.reload_handle),
                Arc::clone(&parts.session),
                crate::LlmKeyStore::new(self.services.secret_store()),
            ),
        ))
    }

    /// Read-only reader exposing the live active/default model id so the WebUI
    /// facade can price a default-model run (one with no `resolved_model_route`)
    /// against the model that actually ran. Backed by the same hot-swappable
    /// primary provider the model gateway drives, so it tracks operator model
    /// swaps. `None` when no LLM provider was wired at boot.
    pub(crate) fn webui_active_model_reader(
        &self,
    ) -> Option<Arc<dyn ironclaw_product_workflow::ActiveModelReader>> {
        let parts = self.llm_reload.as_ref()?;
        Some(Arc::new(
            crate::llm_admin::active_model::ProviderActiveModelReader::new(
                parts.reload_handle.primary_provider(),
            ),
        ))
    }

    /// Diagnostic id for the no-profile run profile selected by this runtime.
    pub fn default_run_profile_id(&self) -> &str {
        &self.default_run_profile_id
    }

    /// Test-only accessor for the composition-owned trigger repository so
    /// integration tests can seed `TriggerRecord` rows that the spawned
    /// trigger poller will observe through its production read path. Returns
    /// `None` when the runtime was built without a local-runtime substrate
    /// (e.g. production-shape profiles that haven't been wired end-to-end
    /// yet). Gated behind `test-support` so the substrate handle never leaks
    /// into production builds. Mirrors the production read path exercised by
    /// the spawned trigger poller worker, which calls
    /// `TriggerRepository::list_due_triggers` on every tick and the
    /// per-trigger `claim_due_fire` / `mark_fire_*` mutation methods.
    #[cfg(any(test, feature = "test-support"))]
    pub fn trigger_repository(&self) -> Option<Arc<dyn ironclaw_triggers::TriggerRepository>> {
        self.services
            .local_runtime
            .as_ref()
            .map(|local_runtime| Arc::clone(&local_runtime.trigger_repository))
    }

    /// Test-only accessor for the trigger repository backing a
    /// production-shaped runtime (where `local_runtime` is `None`, so
    /// [`Self::trigger_repository`] returns `None`). Mirrors the production
    /// call site `RebornProductionRuntimeServices::trigger_repository`, which
    /// the WebUI automations facade uses. Integration tests use this to seed
    /// due `TriggerRecord` rows on a production runtime before driving the
    /// poller. Returns `None` when no production store graph is present. Gated
    /// behind `test-support` so the substrate handle never leaks into
    /// production builds. For tests only.
    #[cfg(any(test, feature = "test-support"))]
    pub fn production_trigger_repository_for_test(
        &self,
    ) -> Option<Arc<dyn ironclaw_triggers::TriggerRepository>> {
        self.services
            .production_runtime
            .as_ref()
            .map(|production| production.trigger_repository())
    }

    /// Test-only accessor for the SAME `ConversationActorPairingService`
    /// instance the spawned trigger poller's
    /// [`ConversationContentRefMaterializer`] consults. Integration tests
    /// use this to call the production `pair_external_actor` API and seed
    /// the trigger creator's actor pairing — without it, the materializer
    /// fails closed with `BindingRequired` (by design: trigger fires never
    /// auto-pair unknown actors). Returns `None` when the trigger poller
    /// wasn't built for this runtime (poller disabled). Gated behind
    /// `test-support` so the conversation handle never leaks into
    /// production builds.
    #[cfg(any(test, feature = "test-support"))]
    pub fn trigger_conversation_pairing(
        &self,
    ) -> Option<Arc<dyn ironclaw_conversations::ConversationActorPairingService>> {
        self.trigger_conversation_pairing.as_ref().map(Arc::clone)
    }

    /// Open the canonical Reborn identity resolver on the runtime's existing
    /// local-dev libSQL substrate handle, running the store's idempotent
    /// schema migrations plus the one-time legacy WebUI identity fold under
    /// `tenant_id`. Rides the same `reborn-local-dev.db` handle the runtime
    /// already owns rather than opening a second handle to that file (the
    /// host filesystem abstraction owns the substrate, not the caller).
    /// Construct the canonical identity store over a host scoped filesystem.
    /// Shared by the local-runtime and production-shaped accessors: the store is
    /// generic over the backend filesystem `F`, tenant-partitions records by the
    /// per-call tenant in the path, and takes the runtime-owner caller identity as
    /// its fixed host-API scope.
    fn identity_store_over<F>(
        &self,
        scoped_filesystem: Arc<ironclaw_filesystem::ScopedFilesystem<F>>,
    ) -> ironclaw_reborn_identity::FilesystemRebornIdentityStore<F>
    where
        F: RootFilesystem + 'static,
    {
        ironclaw_reborn_identity::FilesystemRebornIdentityStore::new(
            scoped_filesystem,
            self.thread_scope.tenant_id.clone(),
            self.actor_user_id.clone(),
            self.thread_scope.agent_id.clone(),
            self.thread_scope.project_id.clone(),
        )
    }

    /// Open the SSO/admin identity resolver over the host-owned identity substrate.
    /// `None` only when the runtime has no durable substrate at all (neither a
    /// local-runtime nor a production store graph), so callers fail closed instead
    /// of synthesizing a second identity store. Greenfield: no legacy fold runs on
    /// this tree (blank-slate deploy). See #5013.
    pub async fn open_reborn_identity_resolver(
        &self,
        _tenant_id: &TenantId,
    ) -> Option<
        Result<
            Arc<dyn ironclaw_reborn_identity::RebornIdentityResolver>,
            ironclaw_reborn_identity::RebornIdentityError,
        >,
    > {
        // Local-runtime substrate: build the store on the host scoped filesystem
        // (same substrate boundary as every other durable store), scoped by the
        // runtime-owner caller identity. Data is partitioned by tenant in the
        // record path.
        if let Some(local) = self.services.local_runtime.as_ref() {
            let store = self.identity_store_over(Arc::clone(&local.identity_filesystem));
            return Some(Ok(
                Arc::new(store) as Arc<dyn ironclaw_reborn_identity::RebornIdentityResolver>
            ));
        }

        // Production-shaped substrate (#5013): the identity store lives on the same
        // `/tenant-shared/reborn-identity` mount the local build uses — production's
        // `wrap_scoped` installs the identical `invocation_mount_view` resolver, which
        // rewrites the prefix by the runtime-owner scope's tenant and treats the
        // per-call tenant as an opaque path tail, so one owner-scoped store serves
        // every tenant.
        if let Some(production) = self.services.production_runtime.as_ref() {
            let store: Arc<dyn ironclaw_reborn_identity::RebornIdentityResolver> = match production
            {
                crate::factory::RebornProductionRuntimeServices::LibSql(graph) => {
                    Arc::new(self.identity_store_over(Arc::clone(&graph.scoped_filesystem)))
                }
                crate::factory::RebornProductionRuntimeServices::Postgres(graph) => {
                    Arc::new(self.identity_store_over(Arc::clone(&graph.scoped_filesystem)))
                }
            };
            return Some(Ok(store));
        }
        None
    }

    /// Open the admin user-directory surface over the host-owned identity
    /// substrate. Same store [`open_reborn_identity_resolver`] uses
    /// (`FilesystemRebornIdentityStore` implements both traits), so admin CRUD
    /// enumerates exactly the users SSO login persists. `None` only when the
    /// runtime has no durable substrate at all — neither a local-runtime nor a
    /// production store graph (fail closed). Synchronous and fold-free (the legacy
    /// fold seeds identity/index records, not `StoredUser` rows the directory
    /// reads), so `build_webui_services` can call it directly.
    pub(crate) fn reborn_user_directory(
        &self,
    ) -> Option<Arc<dyn ironclaw_reborn_identity::RebornUserDirectory>> {
        if let Some(local) = self.services.local_runtime.as_ref() {
            return Some(
                Arc::new(self.identity_store_over(Arc::clone(&local.identity_filesystem)))
                    as Arc<dyn ironclaw_reborn_identity::RebornUserDirectory>,
            );
        }
        // Production-shaped substrate (#5013): same identity mount and per-call
        // tenant partitioning as the local path — see `open_reborn_identity_resolver`.
        if let Some(production) = self.services.production_runtime.as_ref() {
            let directory: Arc<dyn ironclaw_reborn_identity::RebornUserDirectory> = match production
            {
                crate::factory::RebornProductionRuntimeServices::LibSql(graph) => {
                    Arc::new(self.identity_store_over(Arc::clone(&graph.scoped_filesystem)))
                }
                crate::factory::RebornProductionRuntimeServices::Postgres(graph) => {
                    Arc::new(self.identity_store_over(Arc::clone(&graph.scoped_filesystem)))
                }
            };
            return Some(directory);
        }
        None
    }

    /// Test-only accessor for the admin user directory the WebUI facade wires.
    /// Mirrors the production call `build_webui_services` makes to
    /// [`Self::reborn_user_directory`] (`pub(crate)`), which integration tests
    /// in a separate crate cannot reach. Gated behind `test-support` so the
    /// substrate handle never leaks into production builds. For tests only.
    #[cfg(any(test, feature = "test-support"))]
    pub fn reborn_user_directory_for_tests(
        &self,
    ) -> Option<Arc<dyn ironclaw_reborn_identity::RebornUserDirectory>> {
        self.reborn_user_directory()
    }

    /// Admin per-user secret provisioner over the host-owned secret substrate,
    /// scoped to an arbitrary target user (not the runtime owner). `None` only
    /// when no durable secret store was built (a no-storage local build).
    /// Production-shaped builds source it from the production store graph.
    /// See `admin_secrets.rs`.
    pub(crate) fn reborn_admin_secret_provisioner(
        &self,
    ) -> Option<Arc<dyn crate::admin_secrets::AdminSecretProvisioner>> {
        if let Some(local) = self.services.local_runtime.as_ref() {
            return local.admin_secret_provisioner.clone();
        }
        // Production-shaped substrate: the provisioner was built over the raw
        // production root + the runtime's own crypto in `build_backend_production`.
        if let Some(production) = self.services.production_runtime.as_ref() {
            return Some(match production {
                crate::factory::RebornProductionRuntimeServices::LibSql(graph) => {
                    Arc::clone(&graph.admin_secret_provisioner)
                }
                crate::factory::RebornProductionRuntimeServices::Postgres(graph) => {
                    Arc::clone(&graph.admin_secret_provisioner)
                }
            });
        }
        None
    }

    /// First-class projects + membership (ACL) facade over the host-owned scoped
    /// substrate, backing the WebUI project surface. `None` only when the runtime
    /// has no durable substrate at all (neither a local-runtime nor a production
    /// store graph). Production-shaped builds source it from the production graph.
    pub(crate) fn reborn_project_service(
        &self,
    ) -> Option<Arc<dyn ironclaw_product_workflow::ProjectService>> {
        if let Some(local) = self.services.local_runtime.as_ref() {
            return Some(Arc::clone(&local.project_service));
        }
        if let Some(production) = self.services.production_runtime.as_ref() {
            return Some(match production {
                crate::factory::RebornProductionRuntimeServices::LibSql(graph) => {
                    Arc::clone(&graph.project_service)
                }
                crate::factory::RebornProductionRuntimeServices::Postgres(graph) => {
                    Arc::clone(&graph.project_service)
                }
            });
        }
        None
    }

    /// The admin API-token minter supplied via
    /// [`RebornRuntimeInput::with_admin_api_token_minter`], if any.
    pub(crate) fn reborn_admin_token_minter(&self) -> Option<Arc<dyn crate::AdminApiTokenMinter>> {
        self.admin_api_token_minter.clone()
    }

    pub(crate) fn webui_thread_service(&self) -> Arc<dyn SessionThreadService> {
        self.thread_service.clone()
    }

    /// Test-only accessor for the session thread service shared by the trigger
    /// poller, REPL, and WebUI paths. Integration tests use this to enumerate
    /// threads stored by `record_trigger_prompt` without going through the WebUI
    /// `/api/webchat/v2/threads` endpoint (which filters automation threads out
    /// of the list response). The returned handle is the same `Arc` the
    /// production code uses; writes made through it are visible to all paths.
    #[cfg(any(test, feature = "test-support"))]
    pub fn session_thread_service(&self) -> Arc<dyn ironclaw_threads::SessionThreadService> {
        Arc::clone(&self.thread_service)
    }

    pub(crate) fn webui_turn_coordinator(&self) -> Arc<dyn TurnCoordinator> {
        self.turn_coordinator.clone()
    }

    /// The runtime's turn coordinator — the same `Arc` production wiring hands
    /// to the WebUI facade and the channel hosts
    /// ([`RebornRuntime::webui_turn_coordinator`]) — so downstream integration
    /// tests can poll `GetRunStateRequest` for runs submitted through the
    /// composed surfaces (e.g. waiting on a `BlockedAuth` park and its resume).
    /// For tests only — ships zero bytes in production builds.
    #[cfg(any(test, feature = "test-support"))]
    pub fn webui_turn_coordinator_for_test(&self) -> Arc<dyn TurnCoordinator> {
        self.webui_turn_coordinator()
    }

    /// The generic post-OAuth channel-identity binding config for this
    /// deployment (extension-runtime §5.5): channel extensions bind through
    /// generic discovery over the durable installation store; bindings
    /// persist in the generic channel-identity store; post-bind DM-target
    /// provisioning opens the caller's direct conversation through the
    /// extension's own adapter. `None` when the composed runtime carries no
    /// durable channel-identity storage.
    /// The bearer-authed generic pairing route mount (`WebGeneratedCode`
    /// channels), when the composed runtime built any pairing service.
    pub fn channel_pairing_route_mount(
        &self,
    ) -> Option<crate::webui::route_mounts::ProtectedRouteMount> {
        self.services.channel_pairing.as_ref().map(|registry| {
            crate::extension_host::channel_pairing_serve::channel_pairing_route_mount(
                std::sync::Arc::clone(registry),
            )
        })
    }

    pub fn channel_identity_binding_config(
        &self,
    ) -> Option<crate::extension_host::channel_identity::ChannelIdentityBindingConfig> {
        let local_runtime = self.services.local_runtime.as_ref()?;
        let identity_store = local_runtime.channel_identity_store.clone()?;
        let installation_store = local_runtime
            .extension_management
            .as_ref()
            .map(|management| management.installation_store_handle());
        let snapshot_updates = local_runtime
            .extension_management
            .as_ref()
            .and_then(|management| management.generic_host())
            .map(|host| host.snapshot_watch().subscribe());
        let post_bind_factory = match (
            self.services.channel_delivery_resolver(),
            local_runtime.channel_dm_target_store.clone(),
            snapshot_updates,
        ) {
            (Some(delivery), Some(store), Some(snapshot_updates)) => Some(Arc::new(
                crate::extension_host::channel_dm_provisioning::ChannelDmTargetProvisioning::new(
                    delivery,
                    store,
                    snapshot_updates,
                ),
            )
                as Arc<
                    dyn crate::extension_host::channel_identity::ChannelIdentityPostBindFactory,
                >),
            _ => None,
        };
        Some(
            crate::extension_host::channel_identity::ChannelIdentityBindingConfig {
                tenant_id: self.thread_scope.tenant_id.clone(),
                installation_store,
                channel_config: local_runtime.channel_config.clone(),
                binding_store: Arc::clone(&identity_store)
                    as Arc<dyn crate::provider_identity::RebornUserIdentityBindingStore>,
                rollback_store: identity_store
                    as Arc<dyn crate::provider_identity::RebornUserIdentityBindingDeleteStore>,
                post_bind_factory,
                overrides: Vec::new(),
            },
        )
    }

    /// The generic per-user channel-connection facade over the same generic
    /// stores (discovery from the installation store; connected = an
    /// identity binding under the extension's installation prefix;
    /// disconnect clears bindings, vendor credentials, and the provisioned
    /// DM target). `None` when the composed runtime carries no durable
    /// channel-identity storage.
    pub(crate) fn generic_channel_connection_facade(
        &self,
    ) -> Option<Arc<dyn ironclaw_product_workflow::ChannelConnectionFacade>> {
        let local_runtime = self.services.local_runtime.as_ref()?;
        let identity_store = local_runtime.channel_identity_store.clone()?;
        let installation_store = local_runtime
            .extension_management
            .as_ref()
            .map(|management| management.installation_store_handle());
        let credential_cleanup = self.services.product_auth.clone().map(|services| {
            services as Arc<dyn crate::extension_host::channel_connection::ChannelCredentialCleanup>
        });
        let account_status_reader = self.services.product_auth.clone().map(|services| {
            services
                as Arc<dyn crate::extension_host::channel_connection::ChannelAccountStatusReader>
        });
        Some(Arc::new(
            crate::extension_host::channel_connection::GenericChannelConnectionFacade::new(
                self.thread_scope.tenant_id.clone(),
                Vec::new(),
                installation_store,
                Arc::clone(&identity_store)
                    as Arc<dyn crate::provider_identity::RebornUserIdentityLookup>,
                identity_store
                    as Arc<dyn crate::provider_identity::RebornUserIdentityBindingDeleteStore>,
                credential_cleanup,
                account_status_reader,
                local_runtime.channel_dm_target_store.clone(),
                self.services.channel_pairing.clone(),
            ),
        ))
    }

    pub(crate) fn webui_event_stream(&self) -> Arc<dyn ProjectionStream> {
        self.projection_services.webui_event_stream()
    }

    pub(crate) fn webui_approval_interaction_service(&self) -> Arc<dyn ApprovalInteractionService> {
        self.approval_interaction_service.clone()
    }

    pub(crate) fn webui_auth_interaction_service(&self) -> Arc<dyn AuthInteractionService> {
        self.auth_interaction_service.clone()
    }

    pub(crate) fn outbound_delivery_target_provider(
        &self,
    ) -> Option<Arc<dyn OutboundDeliveryTargetProvider>> {
        self.outbound_delivery_target_registry
            .as_ref()
            .map(|registry| {
                let registry = Arc::clone(registry);
                let provider: Arc<dyn OutboundDeliveryTargetProvider> = registry;
                provider
            })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn register_outbound_delivery_target_provider(
        &self,
        provider_key: impl Into<String>,
        provider: Arc<dyn OutboundDeliveryTargetProvider>,
    ) -> Result<OutboundDeliveryTargetRegistrationOutcome, RebornRuntimeError> {
        let Some(registry) = self.outbound_delivery_target_registry.as_ref() else {
            return Err(RebornRuntimeError::InvalidArgument {
                reason: "outbound delivery target registry unavailable for this runtime"
                    .to_string(),
            });
        };
        registry
            .register_provider(provider_key, provider)
            .map_err(|error| RebornRuntimeError::InvalidArgument {
                reason: format!("outbound delivery target provider registration failed: {error}"),
            })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn register_static_outbound_delivery_target_for_test(
        &self,
        provider_key: impl Into<String>,
        target_id: RebornOutboundDeliveryTargetId,
        channel: &str,
        display_name: &str,
        description: Option<&str>,
        reply_target_binding_ref: ReplyTargetBindingRef,
    ) -> Result<(), RebornRuntimeError> {
        let target_id = OutboundDeliveryTargetId::new(target_id.as_str()).map_err(|error| {
            RebornRuntimeError::InvalidArgument {
                reason: format!("invalid outbound delivery target id: {error}"),
            }
        })?;
        let summary = OutboundDeliveryTargetSummary::new(
            target_id,
            channel,
            display_name,
            description.map(ToOwned::to_owned),
        )
        .map_err(|error| RebornRuntimeError::InvalidArgument {
            reason: format!("invalid outbound delivery target summary: {error}"),
        })?;
        self.register_outbound_delivery_target_provider(
            provider_key,
            Arc::new(StaticOutboundDeliveryTargetProvider {
                summary,
                capabilities: DeliveryTargetCapabilities {
                    final_replies: true,
                    progress: false,
                    gate_prompts: false,
                    auth_prompts: false,
                    modalities: Vec::new(),
                },
                reply_target_binding_ref,
            }),
        )
        .map(|_| ())
    }

    #[cfg(test)]
    fn webui_approval_audit_sink(&self) -> Arc<InMemoryAuditSink> {
        self.approval_audit_sink.clone()
    }

    pub(crate) fn webui_skill_activation_source(
        &self,
    ) -> Option<Arc<ComposedSelectableSkillContextSource>> {
        self.skill_activation_source.clone()
    }

    /// Read-write project-scoped workspace filesystem for landing inbound
    /// attachment bytes at paths the agent's file tools can later read back.
    /// `None` when no local runtime is composed.
    ///
    /// This deliberately does NOT reuse `rt.workspace_filesystem`: that handle
    /// is intentionally read-only (it backs setup-marker reads — see
    /// `local_dev_setup_marker_workspace_filesystem_is_read_only`), so writing
    /// an attachment through it fails closed with `PermissionDenied`. Delegates
    /// to `RebornServices::read_write_workspace_filesystem` — the single owner
    /// of this recipe, shared with the `local_dev_attachment_test_support_for_test`
    /// C-ATTACH test seam so the two views can never drift apart.
    pub(crate) fn webui_workspace_filesystem(
        &self,
    ) -> Option<
        Arc<ironclaw_filesystem::ScopedFilesystem<ironclaw_filesystem::CompositeRootFilesystem>>,
    > {
        self.services.read_write_workspace_filesystem()
    }

    /// Read-only scoped filesystem spanning every mount the standalone WebUI
    /// filesystem viewer can browse (workspace files + persistent memory), over
    /// the same composite root the agent's tools resolve through. `None` only
    /// when no local runtime is composed; scope-specific mount resolution errors
    /// surface during browse operations.
    ///
    /// Distinct from [`Self::webui_workspace_filesystem`]: that handle is the
    /// read-write workspace-only view used to land attachments, whereas this is
    /// a strictly read-only, multi-mount navigation view.
    pub(crate) fn webui_browse_filesystem(
        &self,
    ) -> Option<
        Arc<ironclaw_filesystem::ScopedFilesystem<ironclaw_filesystem::CompositeRootFilesystem>>,
    > {
        let rt = self.services.local_runtime.as_ref()?;
        Some(Arc::new(ironclaw_filesystem::ScopedFilesystem::new(
            Arc::clone(&rt.extension_filesystem),
            crate::local_dev_mounts::scoped_browse_mount_view,
        )))
    }

    /// Test-only handle on the resource governor backing the budget
    /// accountant. Exposed under `test-support` so integration tests can
    /// assert ledger state after a `send_user_message` round-trip.
    #[cfg(any(test, feature = "test-support"))]
    pub fn budget_resource_governor(
        &self,
    ) -> Option<Arc<dyn ironclaw_resources::ResourceGovernor>> {
        self.services
            .local_runtime
            .as_ref()
            .map(|rt| Arc::clone(&rt.resource_governor))
    }

    /// Test-only handle on the in-memory budget event sink wired to the
    /// governor. Tests use `.drain()` / `.snapshot()` to inspect the
    /// audit-event stream produced by a run.
    #[cfg(any(test, feature = "test-support"))]
    pub fn budget_event_sink(&self) -> Option<Arc<ironclaw_resources::InMemoryBudgetEventSink>> {
        self.services
            .local_runtime
            .as_ref()
            .map(|rt| Arc::clone(&rt.in_memory_budget_event_sink))
    }

    /// Broadcast sink that fans every emitted `BudgetEvent` to any
    /// subscriber. The runtime always spawns its own subscriber — the
    /// [`crate::observability::budget_events::BudgetEventProjection`] task wired by
    /// `build_reborn_runtime` and shut down via [`Self::shutdown`] —
    /// so this sink is never a no-op even when the caller does not
    /// install a custom observer (review feedback Thermo-Nuclear #3
    /// / follow-up A2). Callers that need a richer projection
    /// (multi-channel fan-out, telemetry exporters) should pass an
    /// observer through
    /// [`crate::RebornRuntimeInput::with_budget_event_observer`]
    /// rather than re-subscribing here; spawning a second long-lived
    /// receiver risks one of them lagging while the other drains.
    pub fn broadcast_budget_event_sink(
        &self,
    ) -> Option<Arc<ironclaw_resources::BroadcastBudgetEventSink>> {
        self.services
            .local_runtime
            .as_ref()
            .map(|rt| Arc::clone(&rt.broadcast_budget_event_sink))
    }

    /// Test-only handle on the budget approval-gate store. Tests resolve
    /// pending gates here (Approve / Cancel / let-expire) to drive the
    /// F3/F4/F5 approval-flow scenarios.
    #[cfg(any(test, feature = "test-support"))]
    pub fn budget_gate_store(&self) -> Option<Arc<dyn ironclaw_resources::BudgetGateStore>> {
        self.services
            .local_runtime
            .as_ref()
            .map(|rt| Arc::clone(&rt.budget_gate_store))
    }

    /// Test-only lookup scope for budget gates opened by a run in this
    /// conversation. Durable gate stores route by the run's resource scope;
    /// tests must not use `ResourceScope::system()` because the in-memory
    /// store ignores scope while filesystem-backed stores do not.
    #[cfg(any(test, feature = "test-support"))]
    pub fn budget_gate_scope_for_conversation(
        &self,
        conversation: &ConversationId,
    ) -> ironclaw_host_api::ResourceScope {
        self.turn_scope_for(&conversation.0).to_resource_scope()
    }

    /// Test-only: enable the global auto-approve switch for this runtime's
    /// actor scope so a scripted turn exercises the dispatch path instead of
    /// blocking on the per-tool approval gate. The Tools-settings switch is
    /// authoritative for first-party tool dispatch; turning it on here
    /// mirrors what an operator would do before letting the agent run tools.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn enable_global_auto_approve_for_test(&self, conversation: &ConversationId) {
        let store = self
            .services
            .local_dev_auto_approve_settings_for_test()
            .expect("local-dev runtime should expose an auto-approve setting store");
        let scope = self.turn_scope_for(&conversation.0).to_resource_scope();
        store
            .set(ironclaw_approvals::AutoApproveSettingInput {
                updated_by: ironclaw_host_api::Principal::User(scope.user_id.clone()),
                scope,
                enabled: true,
            })
            .await
            .expect("enabling global auto-approve should succeed");
    }

    /// Apply the outcome of a resolved [`BudgetApprovalGate`]: when the
    /// gate is approved, raise the affected account's limit so a
    /// subsequent `send_user_message` can re-issue the reservation that
    /// previously crossed the pause threshold. Returns the resolved
    /// gate.
    ///
    /// Production wires this through a gate-resolution route on the web
    /// gateway; the test-only accessor lets E2E tests drive F3 / F4 / F5
    /// without booting that surface.
    #[cfg(any(test, feature = "test-support"))]
    pub fn apply_resolved_budget_gate(
        &self,
        scope: &ironclaw_host_api::ResourceScope,
        gate_id: ironclaw_resources::BudgetGateId,
    ) -> Result<ironclaw_resources::BudgetApprovalGate, RebornRuntimeError> {
        let local_runtime = self.services.local_runtime.as_ref().ok_or_else(|| {
            RebornRuntimeError::InvalidArgument {
                reason: "local-dev runtime substrate required to apply a budget gate".to_string(),
            }
        })?;
        let gate = local_runtime
            .budget_gate_store
            .get(scope, gate_id)
            .map_err(|error| RebornRuntimeError::InvalidArgument {
                reason: format!("budget gate read failed: {error}"),
            })?
            .ok_or_else(|| RebornRuntimeError::InvalidArgument {
                reason: format!("unknown budget gate: {gate_id}"),
            })?;
        if let ironclaw_resources::BudgetGateStatus::Approved {
            increased_limit, ..
        } = &gate.status
        {
            local_runtime
                .resource_governor
                .set_limit(gate.needed.account.clone(), increased_limit.clone())
                .map_err(|error| RebornRuntimeError::InvalidArgument {
                    reason: format!("failed to apply approved budget limit: {error}"),
                })?;
        }
        Ok(gate)
    }

    /// Create a fresh conversation. Returns the opaque conversation id used
    /// in subsequent `send_user_message` calls.
    ///
    /// The thread is materialized inside the session thread service so
    /// `accept_inbound_message` does not error on the first send.
    pub async fn new_conversation(&self) -> Result<ConversationId, RebornRuntimeError> {
        let thread_id =
            ThreadId::new(format!("reborn-conv-{}", Uuid::new_v4())).map_err(|reason| {
                RebornRuntimeError::InvalidArgument {
                    reason: reason.to_string(),
                }
            })?;
        self.thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: self.thread_scope.clone(),
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: self.actor_user_id.as_str().to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .map_err(|error| RebornRuntimeError::ThreadService(error.to_string()))?;
        Ok(ConversationId(thread_id))
    }

    /// Submit a user message into the conversation, wait for the run to
    /// reach a terminal state, and return the assistant reply read back
    /// from the session thread service.
    ///
    /// Without an LLM provider configured, the run will fail and the
    /// returned reply will surface that failure via `status = Failed`
    /// and `text = None`.
    ///
    /// **WebUI-only origin contract**: this task-level send path resolves
    /// the turn's product-context origin as WebUI chat (`resolve_web_ui`).
    /// A non-WebUI ingress (e.g. a future channel adapter) must not reuse
    /// this method for its submissions; it must resolve its own origin at
    /// that ingress instead.
    pub async fn send_user_message(
        &self,
        conversation: &ConversationId,
        text: &str,
    ) -> Result<AssistantReply, RebornRuntimeError> {
        self.send_user_message_with_cancellation(conversation, text, CancellationToken::new())
            .await
    }

    /// Submit a user message with a cooperative cancellation token. If the
    /// token fires while waiting for completion, the runtime cancels the run
    /// before returning.
    pub async fn send_user_message_with_cancellation(
        &self,
        conversation: &ConversationId,
        text: &str,
        cancellation: CancellationToken,
    ) -> Result<AssistantReply, RebornRuntimeError> {
        self.send_user_message_internal(conversation, text, cancellation, false)
            .await
    }

    async fn send_user_message_internal(
        &self,
        conversation: &ConversationId,
        text: &str,
        cancellation: CancellationToken,
        capture_skill_execution_plan: bool,
    ) -> Result<AssistantReply, RebornRuntimeError> {
        let total_started_at = live_latency_started_at();
        let submit_started_at = total_started_at;
        let submitted = match self
            .submit_user_turn(
                conversation,
                text,
                &cancellation,
                capture_skill_execution_plan,
            )
            .await
        {
            Ok(submitted) => {
                trace_runtime_latency_ok(
                    "submit_user_turn",
                    &conversation.0,
                    Some(submitted.run_id),
                    submit_started_at,
                );
                submitted
            }
            Err(error) => {
                trace_runtime_latency_error(
                    "submit_user_turn",
                    &conversation.0,
                    None,
                    submit_started_at,
                    &error,
                );
                trace_runtime_latency_error(
                    "send_user_message",
                    &conversation.0,
                    None,
                    total_started_at,
                    &error,
                );
                return Err(error);
            }
        };

        let wait_started_at = live_latency_started_at();
        let reply = async {
            let terminal_state = self
                .wait_for_terminal(&submitted.scope, submitted.run_id, &cancellation)
                .await?;
            let assistant_text = self
                .read_latest_assistant_text(&conversation.0, submitted.run_id)
                .await?;

            Ok(AssistantReply {
                conversation: conversation.clone(),
                run_id: submitted.run_id,
                status: terminal_state.status,
                failure_category: terminal_state
                    .failure
                    .as_ref()
                    .map(|failure| failure.category().to_string()),
                text: assistant_text,
            })
        }
        .await;
        match &reply {
            Ok(_) => trace_runtime_latency_ok(
                "wait_for_terminal_and_read_reply",
                &conversation.0,
                Some(submitted.run_id),
                wait_started_at,
            ),
            Err(error) => trace_runtime_latency_error(
                "wait_for_terminal_and_read_reply",
                &conversation.0,
                Some(submitted.run_id),
                wait_started_at,
                error,
            ),
        }

        if let Some(skill_activation_source) = &self.skill_activation_source
            && let Err(clear_error) = skill_activation_source
                .clear_accepted_message(&submitted.scope, &submitted.accepted_message_ref)
        {
            if reply.is_ok() {
                // Primary turn succeeded, so the cleanup failure is the only
                // error to surface.
                trace_runtime_latency_error(
                    "send_user_message",
                    &conversation.0,
                    Some(submitted.run_id),
                    total_started_at,
                    &clear_error,
                );
                return Err(RebornRuntimeError::TurnSubmission(clear_error.to_string()));
            }
            // Primary turn already failed: don't mask it with the cleanup
            // error — log the secondary (sanitized id only) and return the
            // primary. See error-handling.md.
            tracing::debug!(
                accepted_message_ref = submitted.accepted_message_ref.as_str(),
                "failed to clear accepted message after primary turn failure"
            );
        }

        match &reply {
            Ok(_) => trace_runtime_latency_ok(
                "send_user_message",
                &conversation.0,
                Some(submitted.run_id),
                total_started_at,
            ),
            Err(error) => trace_runtime_latency_error(
                "send_user_message",
                &conversation.0,
                Some(submitted.run_id),
                total_started_at,
                error,
            ),
        }
        reply
    }

    /// Submit a user message turn and return once the run is accepted, holding
    /// the per-conversation send lock for the returned `SubmittedTurn`'s
    /// lifetime. Shared by [`Self::send_user_message_internal`] and the
    /// test-support [`Self::send_user_message_until_gate`] so both drive an
    /// identical accept/submit path and differ only in how they wait for the
    /// run to settle.
    async fn submit_user_turn(
        &self,
        conversation: &ConversationId,
        text: &str,
        cancellation: &CancellationToken,
        capture_skill_execution_plan: bool,
    ) -> Result<SubmittedTurn, RebornRuntimeError> {
        let send_lock = self.send_lock_for(conversation).await;
        let send_lock_started_at = live_latency_started_at();
        let _send_guard = send_lock.lock_owned().await;
        trace_runtime_latency_ok(
            "send_lock_wait",
            &conversation.0,
            None,
            send_lock_started_at,
        );
        // Stopped only when every worker has exited; a single crashed worker must not
        // reject submissions while others run.
        if self.turn_scheduler.is_stopped() {
            let error = RebornRuntimeError::WorkerStopped;
            trace_runtime_latency_error(
                "submit_user_turn_preflight",
                &conversation.0,
                None,
                send_lock_started_at,
                &error,
            );
            return Err(error);
        }
        let scope = self.turn_scope_for(&conversation.0);
        let accept_started_at = live_latency_started_at();
        let accepted = match self
            .thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: self.thread_scope.clone(),
                thread_id: conversation.0.clone(),
                actor_id: self.actor_user_id.as_str().to_string(),
                source_binding_id: Some(self.source_binding_ref.as_str().to_string()),
                reply_target_binding_id: Some(self.reply_target_binding_ref.as_str().to_string()),
                // This task-level API does not receive an upstream stable
                // event id, so mint a best-effort unique id scoped to the
                // caller-provided source binding.
                external_event_id: Some(format!(
                    "{}:{}",
                    self.source_binding_ref.as_str(),
                    Uuid::new_v4()
                )),
                content: MessageContent::text(text.to_string()),
            })
            .await
        {
            Ok(accepted) => {
                trace_runtime_latency_ok(
                    "accept_inbound_message",
                    &conversation.0,
                    None,
                    accept_started_at,
                );
                accepted
            }
            Err(error) => {
                trace_runtime_latency_error(
                    "accept_inbound_message",
                    &conversation.0,
                    None,
                    accept_started_at,
                    &error,
                );
                return Err(RebornRuntimeError::ThreadService(error.to_string()));
            }
        };

        let accepted_message_ref = AcceptedMessageRef::new(format!("msg:{}", accepted.message_id))
            .map_err(|reason| RebornRuntimeError::InvalidArgument { reason })?;
        let idempotency_key = IdempotencyKey::new(format!(
            "{}-{}",
            self.source_binding_ref.as_str(),
            Uuid::new_v4()
        ))
        .map_err(|reason| RebornRuntimeError::InvalidArgument { reason })?;

        if capture_skill_execution_plan {
            let adapter = self
                .skill_execution_adapter
                .as_ref()
                .ok_or(RebornRuntimeError::SkillExecutionUnavailable)?;
            let skill_record_started_at = live_latency_started_at();
            if let Err(error) = adapter.record_user_message_for_execution(
                scope.clone(),
                accepted_message_ref.clone(),
                text,
            ) {
                trace_runtime_latency_error(
                    "record_skill_execution_message",
                    &conversation.0,
                    None,
                    skill_record_started_at,
                    &error,
                );
                return Err(RebornRuntimeError::TurnSubmission(error.to_string()));
            }
            trace_runtime_latency_ok(
                "record_skill_execution_message",
                &conversation.0,
                None,
                skill_record_started_at,
            );
        } else if let Some(skill_activation_source) = &self.skill_activation_source {
            let skill_record_started_at = live_latency_started_at();
            if let Err(error) = skill_activation_source.record_user_message(
                scope.clone(),
                accepted_message_ref.clone(),
                text,
            ) {
                trace_runtime_latency_error(
                    "record_skill_activation_message",
                    &conversation.0,
                    None,
                    skill_record_started_at,
                    &error,
                );
                return Err(RebornRuntimeError::TurnSubmission(error.to_string()));
            }
            trace_runtime_latency_ok(
                "record_skill_activation_message",
                &conversation.0,
                None,
                skill_record_started_at,
            );
        }

        let turn_submit_started_at = live_latency_started_at();
        let response = match self
            .turn_coordinator
            .submit_turn(SubmitTurnRequest {
                requested_model: None,
                scope: scope.clone(),
                actor: TurnActor::new(self.actor_user_id.clone()),
                accepted_message_ref: accepted_message_ref.clone(),
                source_binding_ref: self.source_binding_ref.clone(),
                reply_target_binding_ref: self.reply_target_binding_ref.clone(),
                requested_run_profile: None,
                idempotency_key,
                received_at: Utc::now(),
                requested_run_id: None,
                parent_run_id: None,
                subagent_depth: 0,
                spawn_tree_root_run_id: None,
                product_context: Some(ironclaw_product_context::resolve_web_ui(
                    scope.product_owner(&TurnActor::new(self.actor_user_id.clone())),
                )),
            })
            .await
        {
            Ok(response) => {
                let SubmitTurnResponse::Accepted { run_id, .. } = &response;
                trace_runtime_latency_ok(
                    "turn_coordinator_submit_turn",
                    &conversation.0,
                    Some(*run_id),
                    turn_submit_started_at,
                );
                response
            }
            Err(error) => {
                trace_runtime_latency_error(
                    "turn_coordinator_submit_turn",
                    &conversation.0,
                    None,
                    turn_submit_started_at,
                    &error,
                );
                if let Some(skill_activation_source) = &self.skill_activation_source {
                    skill_activation_source
                        .clear_accepted_message(&scope, &accepted_message_ref)
                        .map_err(|clear_error| {
                            RebornRuntimeError::TurnSubmission(clear_error.to_string())
                        })?;
                }
                return Err(error.into());
            }
        };

        let SubmitTurnResponse::Accepted {
            run_id,
            status: submit_status,
            event_cursor: submit_cursor,
            ..
        } = response;
        if cancellation.is_cancelled() {
            if let Some(skill_activation_source) = &self.skill_activation_source {
                skill_activation_source
                    .clear_accepted_message(&scope, &accepted_message_ref)
                    .map_err(|error| RebornRuntimeError::TurnSubmission(error.to_string()))?;
            }
            self.cancel_run(
                &scope,
                run_id,
                SanitizedCancelReason::UserRequested,
                "caller-cancel",
            )
            .await?;
            return Err(RebornRuntimeError::OperationCancelled);
        }
        let notify_started_at = live_latency_started_at();
        self.turn_scheduler.notify(TurnRunWake {
            scope: scope.clone(),
            run_id,
            status: submit_status,
            event_cursor: submit_cursor,
        });
        trace_runtime_latency_ok(
            "turn_scheduler_notify",
            &conversation.0,
            Some(run_id),
            notify_started_at,
        );

        Ok(SubmittedTurn {
            _send_guard,
            scope,
            run_id,
            accepted_message_ref,
        })
    }

    /// Submit a skill-aware message through the normal Reborn loop and return
    /// the structured activation plan produced during prompt construction.
    pub async fn execute_skill_message(
        &self,
        conversation: &ConversationId,
        text: &str,
    ) -> Result<RebornSkillExecutionResult, RebornRuntimeError> {
        let adapter = self
            .skill_execution_adapter
            .as_ref()
            .ok_or(RebornRuntimeError::SkillExecutionUnavailable)?;
        let scope = self.turn_scope_for(&conversation.0);
        let reply = self
            .send_user_message_internal(conversation, text, CancellationToken::new(), true)
            .await?;
        let plan = self.skill_execution_plan_for_run(adapter, &scope, reply.run_id)?;
        Ok(RebornSkillExecutionResult { plan, reply })
    }

    /// Read a bundle-relative asset from a skill activated by
    /// [`Self::execute_skill_message`].
    pub async fn read_skill_execution_asset(
        &self,
        conversation: &ConversationId,
        plan: &RebornSkillExecutionPlan,
        activation: &RebornSkillActivation,
        path: impl AsRef<str>,
    ) -> Result<RebornSkillAsset, RebornRuntimeError> {
        if plan.run_context().thread_id != conversation.0 {
            return Err(RebornRuntimeError::SkillExecution(
                "skill execution plan does not belong to this conversation".to_string(),
            ));
        }
        let adapter = self
            .skill_execution_adapter
            .as_ref()
            .ok_or(RebornRuntimeError::SkillExecutionUnavailable)?;
        adapter
            .read_file_for_activation(
                plan.run_context(),
                plan.first_party_plan(),
                &activation.to_first_party_request(),
                path,
            )
            .await
            .map(RebornSkillAsset::from)
            .map_err(skill_asset_error)
    }

    /// Stop the turn-runner worker and the budget-event projection.
    /// Awaits both tasks before returning so background state is fully
    /// drained when the runtime drops.
    pub async fn shutdown(self) -> Result<(), RebornRuntimeError> {
        if let Some(trigger_poller) = self.trigger_poller_handle {
            trigger_poller
                .shutdown(TRIGGER_POLLER_SHUTDOWN_TIMEOUT)
                .await;
        }
        if let Some(credential_refresh_worker) = self.credential_refresh_worker_handle {
            credential_refresh_worker
                .shutdown(ironclaw_auth::KEEPALIVE_SWEEP_SHUTDOWN_TIMEOUT)
                .await;
        }
        self.trace_flush_worker.shutdown().await;
        if let Some(skill_learning_extraction_tasks) = self.skill_learning_extraction_tasks {
            skill_learning_extraction_tasks.shutdown().await;
        }
        self.turn_scheduler.shutdown().await;
        if let Some(projection) = self.budget_event_projection {
            projection.shutdown().await;
        }
        // Everything that mutates turn state (trigger poller, credential-refresh
        // worker, scheduler/runner) is now stopped, so the row store is quiescent.
        // Drain the write-behind durable tail — awaiting the acks of non-critical
        // transitions that committed at memory speed — so a planned restart
        // recovers in-flight turns, not just the synchronously-durable
        // gate-park/terminal/new-run ones. Best-effort: a drain failure means the
        // flusher latched degraded mid-shutdown; log it so the operator sees the
        // un-drained tail rather than failing the clean exit path.
        if let Some(turn_state) = &self.turn_state_flush
            && let Err(error) = turn_state.drain().await
        {
            tracing::warn!(
                %error,
                "turn-state WriteBehind drain failed during graceful shutdown; the un-acked \
                 non-critical tail may not be durable on restart"
            );
        }
        Ok(())
    }

    fn turn_scope_for(&self, thread_id: &ThreadId) -> TurnScope {
        // RebornRuntime is bound to a single actor user, so its turns are
        // owned by that user (not the shared agent).  Passing the explicit
        // owner here makes `TurnScope::product_owner` resolve to
        // `TurnOwner::Personal` instead of `TurnOwner::SharedAgent`.
        TurnScope::new_with_owner(
            self.thread_scope.tenant_id.clone(),
            Some(self.thread_scope.agent_id.clone()),
            self.thread_scope.project_id.clone(),
            thread_id.clone(),
            Some(self.actor_user_id.clone()),
        )
    }

    fn skill_execution_plan_for_run(
        &self,
        adapter: &SkillExecutionAdapter<FilesystemSkillBundleSource<CompositeRootFilesystem>>,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<RebornSkillExecutionPlan, RebornRuntimeError> {
        adapter
            .take_execution_plan_for_run(scope, run_id)
            .map_err(|error| RebornRuntimeError::SkillExecution(error.to_string()))?
            .map(RebornSkillExecutionPlan::from_first_party)
            .ok_or_else(|| {
                RebornRuntimeError::SkillExecution("skill activation plan unavailable".to_string())
            })
    }

    async fn send_lock_for(&self, conversation: &ConversationId) -> Arc<Mutex<()>> {
        let mut locks = self.send_locks.lock().await;
        Arc::clone(
            locks
                .entry(conversation.clone())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }

    async fn wait_for_terminal(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        cancellation: &CancellationToken,
    ) -> Result<TurnRunState, RebornRuntimeError> {
        let start = std::time::Instant::now();
        loop {
            if self.turn_scheduler.is_stopped() {
                return Err(RebornRuntimeError::WorkerStopped);
            }
            let state = self
                .turn_coordinator
                .get_run_state(GetRunStateRequest {
                    scope: scope.clone(),
                    run_id,
                })
                .await?;
            if state.status.is_terminal() {
                return Ok(state);
            }
            // TurnStatus::RecoveryRequired is now terminal (is_terminal() returns true)
            // so the branch above handles it; no special cancel-to-release-lock is needed.
            if start.elapsed() > self.poll_settings.max_total {
                if let Err(error) = self
                    .cancel_run(
                        scope,
                        run_id,
                        SanitizedCancelReason::Timeout,
                        "timeout-cancel",
                    )
                    .await
                {
                    tracing::debug!(
                        ?error,
                        %run_id,
                        "failed to cancel timed-out run while preserving timeout error"
                    );
                }
                return Err(RebornRuntimeError::RunTimeout {
                    timeout: self.poll_settings.max_total,
                });
            }
            tokio::select! {
                _ = cancellation.cancelled() => {
                    if let Err(error) = self
                        .cancel_run(
                            scope,
                            run_id,
                            SanitizedCancelReason::UserRequested,
                            "caller-cancel",
                        )
                        .await
                    {
                        tracing::debug!(
                            ?error,
                            %run_id,
                            "failed to cancel caller-cancelled run while preserving cancellation error"
                        );
                    }
                    return Err(RebornRuntimeError::OperationCancelled);
                }
                _ = tokio::time::sleep(self.poll_settings.interval) => {}
            }
        }
    }

    /// Like [`Self::wait_for_terminal`], but also returns when the run parks on
    /// a user-/client-resolvable gate (auth/approval/resource/external-tool)
    /// instead of polling until those non-terminal states either resolve or hit
    /// `RunTimeout`.
    /// `BlockedDependentRun` is deliberately excluded — it is an internal wait
    /// on a child run, not facade-resolvable, so it keeps polling. The returned
    /// state carries the `Blocked*` status and
    /// `gate_ref`; the caller decides whether to resolve (through the WebUI
    /// facade) or stop. Test/recording-support only.
    #[cfg(any(test, feature = "test-support"))]
    async fn wait_for_terminal_or_gate(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        cancellation: &CancellationToken,
    ) -> Result<TurnRunState, RebornRuntimeError> {
        let start = std::time::Instant::now();
        loop {
            if self.turn_scheduler.is_stopped() {
                return Err(RebornRuntimeError::WorkerStopped);
            }
            let state = self
                .turn_coordinator
                .get_run_state(GetRunStateRequest {
                    scope: scope.clone(),
                    run_id,
                })
                .await?;
            // Exhaustive on purpose: a new `TurnStatus` variant must force a
            // compile error here rather than silently defaulting to "not a
            // gate". Only the user-/client-resolvable gates
            // (auth/approval/resource/external-tool) short-circuit recording.
            // `BlockedDependentRun` is an internal wait on a child run (the
            // upstream contract names it `AwaitDependentRun`) — it is not
            // resolvable through the gate facade, so it keeps polling like
            // `Queued`/`Running` until the dependent run completes or the poll
            // budget expires.
            let blocked_on_gate = match state.status {
                TurnStatus::BlockedApproval
                | TurnStatus::BlockedAuth
                // External-tool gates are resolved by the API client submitting
                // tool output, not by the runtime — short-circuit the wait and
                // return the parked state instead of polling forever.
                | TurnStatus::BlockedExternalTool
                | TurnStatus::BlockedResource => true,
                TurnStatus::BlockedDependentRun
                | TurnStatus::Queued
                | TurnStatus::Running
                | TurnStatus::CancelRequested
                | TurnStatus::Cancelled
                | TurnStatus::Completed
                | TurnStatus::Failed
                | TurnStatus::RecoveryRequired => false,
            };
            if state.status.is_terminal() || blocked_on_gate {
                return Ok(state);
            }
            if start.elapsed() > self.poll_settings.max_total {
                // Surface the primary `RunTimeout`; a failure of the secondary
                // cancel is logged with a sanitized id only and must not mask
                // it (see error-handling.md). `debug!` not `warn!` per the
                // logging rule — this runtime is REPL/TUI-reachable.
                if self
                    .cancel_run(
                        scope,
                        run_id,
                        SanitizedCancelReason::Timeout,
                        "timeout-cancel",
                    )
                    .await
                    .is_err()
                {
                    tracing::debug!(run_id = %run_id, "failed to cancel run after recorder timeout");
                }
                return Err(RebornRuntimeError::RunTimeout {
                    timeout: self.poll_settings.max_total,
                });
            }
            tokio::select! {
                _ = cancellation.cancelled() => {
                    if self
                        .cancel_run(
                            scope,
                            run_id,
                            SanitizedCancelReason::UserRequested,
                            "caller-cancel",
                        )
                        .await
                        .is_err()
                    {
                        tracing::debug!(run_id = %run_id, "failed to cancel run after caller cancellation");
                    }
                    return Err(RebornRuntimeError::OperationCancelled);
                }
                _ = tokio::time::sleep(self.poll_settings.interval) => {}
            }
        }
    }

    /// Test/recording-support sibling of [`Self::send_user_message`] that
    /// returns when the run first reaches a terminal status *or* parks on a
    /// `Blocked*` gate, rather than waiting only for a terminal status.
    ///
    /// The QA-trace recorder (`tests/support/reborn/qa_trace.rs`) uses this so
    /// an OAuth/approval-gated phrase records the agent's decisions up to the
    /// gate and reports the pause, instead of sitting in the non-terminal
    /// `BlockedAuth` state until `RunTimeout` (a real recorder hang this method
    /// exists to eliminate). This method only *observes* where the run paused;
    /// gate *resolution* stays on the WebUI `ProductSurface` facade
    /// (`resolve_gate`) per the #3094 seam — do not add a resolution path here.
    #[cfg(any(test, feature = "test-support"))]
    pub async fn send_user_message_until_gate(
        &self,
        conversation: &ConversationId,
        text: &str,
    ) -> Result<RebornTurnDriveOutcome, RebornRuntimeError> {
        let cancellation = CancellationToken::new();
        let submitted = self
            .submit_user_turn(conversation, text, &cancellation, false)
            .await?;

        let outcome = async {
            let state = self
                .wait_for_terminal_or_gate(&submitted.scope, submitted.run_id, &cancellation)
                .await?;
            let assistant_text = self
                .read_latest_assistant_text(&conversation.0, submitted.run_id)
                .await?;

            if state.status.is_terminal() {
                Ok(RebornTurnDriveOutcome::Terminal(AssistantReply {
                    conversation: conversation.clone(),
                    run_id: submitted.run_id,
                    status: state.status,
                    failure_category: state
                        .failure
                        .as_ref()
                        .map(|failure| failure.category().to_string()),
                    text: assistant_text,
                }))
            } else {
                // `wait_for_terminal_or_gate` only returns terminal or a
                // user-resolvable gate (auth/approval/resource). The
                // blocked-reason contract guarantees a `gate_ref` for those, so
                // a missing one is an invariant violation — surface it as an
                // error rather than letting it look like a valid outcome.
                let gate_ref = state.gate_ref.clone().ok_or_else(|| {
                    RebornRuntimeError::TurnSubmission(format!(
                        "run parked on {:?} without a gate ref",
                        state.status
                    ))
                })?;
                Ok(RebornTurnDriveOutcome::BlockedOnGate {
                    run_id: submitted.run_id,
                    status: state.status,
                    gate_ref,
                    partial_text: assistant_text,
                })
            }
        }
        .await;

        // Clearing the accepted message is safe even on the `BlockedOnGate`
        // path, where the run is still live and resumable: the inbound message
        // is already consumed during the first prompt build (the skill-context
        // source `take`s it), so this is idempotent cleanup of an
        // already-taken entry, and a later gate-resume rebuilds from the active
        // plan candidates rather than this entry. The QA recorder also discards
        // the runtime immediately after, so nothing resumes here in practice.
        if let Some(skill_activation_source) = &self.skill_activation_source
            && let Err(clear_error) = skill_activation_source
                .clear_accepted_message(&submitted.scope, &submitted.accepted_message_ref)
        {
            if outcome.is_ok() {
                // Primary turn succeeded, so the cleanup failure is the only
                // error to surface.
                return Err(RebornRuntimeError::TurnSubmission(clear_error.to_string()));
            }
            // Primary turn already failed: don't mask it with the cleanup
            // error — log the secondary (sanitized id only) and return the
            // primary. See error-handling.md.
            tracing::debug!(
                accepted_message_ref = submitted.accepted_message_ref.as_str(),
                "failed to clear accepted message after primary turn failure"
            );
        }

        outcome
    }

    async fn cancel_run(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        reason: SanitizedCancelReason,
        idempotency_suffix: &str,
    ) -> Result<CancelRunResponse, RebornRuntimeError> {
        let response = self
            .turn_coordinator
            .cancel_run(CancelRunRequest {
                scope: scope.clone(),
                actor: TurnActor::new(self.actor_user_id.clone()),
                run_id,
                reason,
                idempotency_key: IdempotencyKey::new(format!(
                    "{}-{}-{}",
                    self.source_binding_ref.as_str(),
                    idempotency_suffix,
                    run_id
                ))
                .map_err(|reason| RebornRuntimeError::InvalidArgument { reason })?,
            })
            .await?;
        let cancellation_accepted = matches!(
            response.status,
            TurnStatus::CancelRequested | TurnStatus::Cancelled
        );
        if cancellation_accepted {
            self.append_webui_loop_cancelled(scope, run_id).await?;
        }
        self.turn_scheduler.notify(TurnRunWake {
            scope: scope.clone(),
            run_id: response.run_id,
            status: response.status,
            event_cursor: response.event_cursor,
        });
        if cancellation_accepted {
            self.cancel_descendant_runs(scope, run_id, reason, idempotency_suffix)
                .await?;
        }
        Ok(response)
    }

    async fn cancel_descendant_runs(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        reason: SanitizedCancelReason,
        idempotency_suffix: &str,
    ) -> Result<(), RebornRuntimeError> {
        let mut stack = self.turn_tree_store.children_of(scope, run_id).await?;
        let mut visited = HashSet::new();
        let mut visited_count = 0_usize;
        while let Some(child) = stack.pop() {
            if !visited.insert(child.run_id) {
                continue;
            }
            visited_count += 1;
            if visited_count > MAX_DESCENDANT_CANCEL_NODES {
                tracing::warn!(
                    scope = ?scope,
                    run_id = %run_id,
                    max_nodes = MAX_DESCENDANT_CANCEL_NODES,
                    "stopped descendant cancellation traversal after node budget was reached"
                );
                break;
            }
            if child.status.is_terminal() {
                continue;
            }
            let grandchildren = self
                .turn_tree_store
                .children_of(&child.scope, child.run_id)
                .await?;
            stack.extend(grandchildren);
            let idempotency_key = IdempotencyKey::new(format!(
                "{}-{}-descendant-{}",
                self.source_binding_ref.as_str(),
                idempotency_suffix,
                child.run_id
            ))
            .map_err(|reason| RebornRuntimeError::InvalidArgument { reason })?;
            let child_scope = child.scope.clone();
            let child_run_id = child.run_id;
            let response = self
                .turn_coordinator
                .cancel_run(CancelRunRequest {
                    scope: child_scope.clone(),
                    actor: TurnActor::new(self.actor_user_id.clone()),
                    run_id: child_run_id,
                    reason,
                    idempotency_key,
                })
                .await;
            let response = match response {
                Ok(response) => response,
                Err(error) => {
                    let state = self
                        .turn_coordinator
                        .get_run_state(GetRunStateRequest {
                            scope: child_scope.clone(),
                            run_id: child_run_id,
                        })
                        .await?;
                    if matches!(
                        state.status,
                        TurnStatus::CancelRequested | TurnStatus::Cancelled
                    ) {
                        self.turn_scheduler.notify(TurnRunWake {
                            scope: child_scope,
                            run_id: child_run_id,
                            status: state.status,
                            event_cursor: EventCursor(0),
                        });
                        continue;
                    }
                    return Err(error.into());
                }
            };
            if matches!(
                response.status,
                TurnStatus::CancelRequested | TurnStatus::Cancelled
            ) {
                self.append_webui_loop_cancelled(&child.scope, child_run_id)
                    .await?;
            }
            self.turn_scheduler.notify(TurnRunWake {
                scope: child_scope,
                run_id: response.run_id,
                status: response.status,
                event_cursor: response.event_cursor,
            });
        }
        Ok(())
    }

    async fn append_webui_loop_cancelled(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<(), RebornRuntimeError> {
        let capability_id = CapabilityId::new(LOOP_RUN_CAPABILITY_ID).map_err(|reason| {
            RebornRuntimeError::InvalidArgument {
                reason: format!("loop-run capability id: {reason}"),
            }
        })?;
        self.webui_event_log
            .append(RuntimeEvent::loop_cancelled(
                ResourceScope {
                    tenant_id: scope.tenant_id.clone(),
                    user_id: self.actor_user_id.clone(),
                    agent_id: scope.agent_id.clone(),
                    project_id: scope.project_id.clone(),
                    mission_id: None,
                    thread_id: Some(scope.thread_id.clone()),
                    invocation_id: InvocationId::from_uuid(run_id.as_uuid()),
                },
                capability_id,
            ))
            .await
            .map(|_| ())
            .map_err(|error| RebornRuntimeError::TurnCoordinator(error.to_string()))
    }

    async fn read_latest_assistant_text(
        &self,
        thread_id: &ThreadId,
        run_id: TurnRunId,
    ) -> Result<Option<String>, RebornRuntimeError> {
        let history = self
            .thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: self.thread_scope.clone(),
                thread_id: thread_id.clone(),
            })
            .await
            .map_err(|error| RebornRuntimeError::ThreadService(error.to_string()))?;
        let run_id_str = run_id.to_string();
        let reply = history
            .messages
            .into_iter()
            .rev()
            .find(|message| {
                matches!(message.kind, MessageKind::Assistant)
                    && matches!(message.status, MessageStatus::Finalized)
                    && message.turn_run_id.as_deref() == Some(run_id_str.as_str())
            })
            .and_then(|message| message.content);
        Ok(reply)
    }
}

/// Build and start a Reborn agent runtime.
///
/// On return, the turn-runner worker is already running in the background and
/// the returned `RebornRuntime` is ready to accept `send_user_message` calls.
///
/// **Currently supported profiles:** `RebornCompositionProfile::LocalDev`,
/// `RebornCompositionProfile::LocalDevYolo`,
/// `RebornCompositionProfile::HostedSingleTenant`, and
/// `RebornCompositionProfile::Production` are wired end-to-end here. Production
/// starts only after readiness diagnostics validate that live traffic can be
/// exposed without a partial cutover.
pub async fn build_reborn_runtime(
    input: RebornRuntimeInput,
) -> Result<RebornRuntime, RebornRuntimeError> {
    let RebornRuntimeInput {
        services: services_input,
        llm,
        boot,
        runner,
        tool_disclosure,
        trigger_poller,
        credential_refresh,
        trigger_fire_access_checker,
        trigger_fire_access,
        poll,
        identity,
        default_project_id,
        regex_skill_activation_enabled,
        skill_context_source: configured_skill_context_source,
        hooks: hooks_config,
        budget_defaults,
        budget_event_observer,
        trajectory_observer,
        admin_api_token_minter,
        #[cfg(any(test, feature = "test-support"))]
        model_gateway_override,
        #[cfg(any(test, feature = "test-support"))]
        model_cost_table_override,
        #[cfg(any(test, feature = "test-support"))]
        model_availability_retry_attempts_override,
    } = input;

    let mut services_input = services_input.ok_or(RebornRuntimeError::InvalidArgument {
        reason: "RebornRuntimeInput.services is required".to_string(),
    })?;

    let profile = services_input.profile();
    // The deployment this build assembles, as data (§4.4/§5.6). Every axis
    // below — live-traffic admission, the cutover gate, substrate selection —
    // reads a field on this value instead of re-matching the profile.
    let deployment = services_input.deployment().clone();
    if let Some(reason) = deployment.traffic().live_traffic_refusal(profile) {
        return Err(RebornRuntimeError::InvalidArgument { reason });
    }
    // Capture the resolved policy before `build_reborn_services` consumes the
    // input. Downstream wiring selects enforcement behaviour from resolved
    // policy *values* (§4.4) rather than re-branching on the deployment
    // profile, so the policy has to outlive the services input.
    let runtime_policy =
        services_input
            .runtime_policy()
            .cloned()
            .ok_or(RebornRuntimeError::InvalidArgument {
                reason: "RebornRuntimeInput.services must include a resolved runtime policy"
                    .to_string(),
            })?;

    let validated_identity = validate_runtime_identity(identity)?;
    services_input = services_input.with_local_runtime_identity(
        validated_identity.tenant_id.clone(),
        validated_identity.agent_id.clone(),
    );
    let mut has_nearai_mcp_bootstrap_config = services_input.has_nearai_mcp_bootstrap_config();
    if !has_nearai_mcp_bootstrap_config
        && let Some(llm) = llm.as_ref()
        && let Some(config) =
            crate::llm_admin::nearai_mcp::nearai_mcp_bootstrap_config_from_llm_config(&llm.config)
                .await
                .map_err(|error| RebornRuntimeError::InvalidArgument {
                    reason: format!("NEAR AI MCP bootstrap config: {error}"),
                })?
    {
        services_input = services_input.with_nearai_mcp_bootstrap_config(config);
        has_nearai_mcp_bootstrap_config = true;
    }
    let trusted_laptop_access = services_input.grants_trusted_laptop_access();
    let owner_id = services_input.owner_id().to_string();
    // Thread per-user and per-origin concurrency caps from TurnRunnerSettings into the
    // turn-state store. The factory reads these when constructing the store so limits
    // are applied from the very first claim.
    let turn_state_limits = TurnStateStoreLimits {
        max_concurrent_runs_per_user: runner.max_concurrent_runs_per_user,
        max_concurrent_trigger_runs: runner.max_concurrent_trigger_runs,
        max_concurrent_conversation_runs: runner.max_concurrent_conversation_runs,
        ..TurnStateStoreLimits::default()
    };
    services_input = services_input.with_turn_state_store_limits(turn_state_limits);
    let actor_user_id =
        UserId::new(owner_id.clone()).map_err(|reason| RebornRuntimeError::InvalidArgument {
            reason: format!("user id: {reason}"),
        })?;
    let nearai_mcp_owner_scope = ResourceScope {
        tenant_id: validated_identity.tenant_id.clone(),
        user_id: actor_user_id.clone(),
        agent_id: Some(validated_identity.agent_id.clone()),
        project_id: default_project_id.clone(),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    let mut services = build_reborn_services(services_input).await?;
    // The stored key no longer feeds the model gateway here (see the
    // post-construction reload below); the NEAR AI MCP bootstrap check is a
    // separate consumer that inspects `llm.config.nearai.api_key` directly,
    // so it still needs the key overlaid onto a local clone.
    if !has_nearai_mcp_bootstrap_config {
        let llm_for_mcp_bootstrap =
            overlay_stored_llm_key_for_nearai_mcp_bootstrap(llm.clone(), &services).await?;
        bootstrap_nearai_mcp_from_effective_llm(
            &services,
            llm_for_mcp_bootstrap.as_ref(),
            nearai_mcp_owner_scope,
        )
        .await?;
    }
    enforce_runtime_cutover_gate(&deployment, &services.readiness)?;

    // Extract the pre-minted scheduler wake wiring from the production composition path
    // (minted in `build_production_shaped`) so it can be handed to
    // `DefaultPlannedRuntimeParts.scheduler_wake_wiring` below. The local-dev path
    // leaves this `None` and `build_default_planned_runtime` mints its own wiring.
    let production_scheduler_wake = {
        let wiring = services.production_scheduler_wake.take();
        // Production and migration-dry-run mint this in `build_production_shaped` so the
        // `HostRuntimeServices` notifier and the scheduler wake loop share one channel.
        // Fail closed if it is missing rather than let `build_default_planned_runtime`
        // mint a divergent scheduler-local channel (silent contract break).
        check_production_scheduler_wake_wiring(profile, &wiring)?;
        wiring
    };

    let runtime_parts = match deployment.substrate() {
        RuntimeSubstrate::Local => {
            let local_runtime =
                services
                    .local_runtime
                    .as_ref()
                    .ok_or(RebornRuntimeError::InvalidArgument {
                        reason: "local-dev RebornServices did not provide runtime substrate"
                            .to_string(),
                    })?;
            local_runtime_parts(local_runtime)
        }
        RuntimeSubstrate::ProductionShaped => {
            let production_runtime = services.production_runtime.as_ref().ok_or(
                RebornRuntimeError::InvalidArgument {
                    reason: "production RebornServices did not provide runtime substrate"
                        .to_string(),
                },
            )?;
            match production_runtime {
                crate::factory::RebornProductionRuntimeServices::LibSql(graph) => {
                    production_runtime_parts(graph)
                }
                crate::factory::RebornProductionRuntimeServices::Postgres(graph) => {
                    production_runtime_parts(graph)
                }
            }
        }
        // `RuntimeSubstrate::None` never reaches here: the disabled
        // deployment's traffic policy refuses live traffic before the services
        // are built, and again at the cutover gate above.
        RuntimeSubstrate::None => {
            return Err(RebornRuntimeError::InvalidArgument {
                reason: format!(
                    "profile={} assembles no runtime substrate",
                    deployment.profile()
                ),
            });
        }
    };
    let RuntimeStoreParts {
        local_runtime,
        turn_state_store,
        checkpoint_state_store,
        loop_checkpoint_store,
        thread_service,
        event_log,
        audit_log,
        resource_governor,
        budget_gate_store,
        broadcast_budget_event_sink,
        subagent_goal_store,
        subagent_await_edge_writer,
        subagent_await_edge_settler,
        subagent_await_edge_evidence,
        trigger_repository,
        turn_run_snapshot_source,
    } = runtime_parts;
    let (skill_context_source, skill_activation_source, skill_execution_adapter) =
        match (configured_skill_context_source, local_runtime) {
            (Some(source), _) => (Some(source), None, None),
            (None, Some(local_runtime)) => {
                let local_dev_skills = local_dev_filesystem_skill_context_source(
                    local_runtime,
                    &validated_identity.tenant_id,
                    regex_skill_activation_enabled,
                )?;
                let skill_warm_scope = ResourceScope {
                    tenant_id: validated_identity.tenant_id.clone(),
                    user_id: actor_user_id.clone(),
                    agent_id: Some(validated_identity.agent_id.clone()),
                    project_id: default_project_id.clone(),
                    mission_id: None,
                    thread_id: None,
                    invocation_id: InvocationId::new(),
                };
                local_dev_skills
                    .bundle_source
                    .warm_system_root_descriptor_cache(&skill_warm_scope)
                    .await
                    .map_err(|error| RebornRuntimeError::InvalidArgument {
                        reason: format!("first-party skills warmup: {error}"),
                    })?;
                (
                    Some(local_dev_skills.source),
                    Some(local_dev_skills.activation_source),
                    Some(local_dev_skills.execution_adapter),
                )
            }
            (None, None) => (None, None, None),
        };

    let tenant_id = validated_identity.tenant_id.clone();
    let agent_id = validated_identity.agent_id.clone();
    let thread_scope = ThreadScope {
        tenant_id,
        agent_id,
        project_id: default_project_id,
        // Keep local-dev runtime threads aligned with WebUI's owner-scoped
        // facade so both entrypoints drive the same runner/evidence path.
        owner_user_id: Some(actor_user_id.clone()),
        mission_id: None,
    };

    // A test gateway override short-circuits the production build entirely:
    //    building a real gateway only to discard it wastes startup work (and, on
    //    the cold-boot path, an LLM session manager), which made
    //    timeout-sensitive tests flaky. When no override is set, build normally.
    // Build the (optional) skill-learning provider from the resolved LLM config.
    // Distillation/refinement runs against a stronger model
    // (IRONCLAW_SKILL_LEARNING_MODEL), reusing the run's NEAR AI credentials
    // with only the model overridden. `llm` no longer feeds the model gateway
    // build below (see `build_production_model_gateway`).
    let skill_learning_provider = match llm.as_ref() {
        Some(resolved) => build_skill_learning_provider(&resolved.config).await,
        None => None,
    };
    // Caller instrumentation seam (e.g. a benchmark harness layering
    // token/reasoning capture): carry the resolved LLM's provider factory into
    // the cold-boot gateway so the wrapper wraps the swappable and stays in the
    // call path across the boot-time reload. `llm` is held by shared reference
    // here (already read above for the NEAR AI MCP bootstrap), so clone the
    // cheap Arc handle rather than move the factory out of the borrow.
    let boot_provider_factory = llm
        .as_ref()
        .and_then(|resolved| resolved.provider_factory.clone());
    #[cfg(any(test, feature = "test-support"))]
    let (model_gateway, llm_cost_table, llm_reload) = match model_gateway_override {
        Some(override_gateway) => (override_gateway, None, None),
        None => build_production_model_gateway(boot_provider_factory).await?,
    };
    #[cfg(not(any(test, feature = "test-support")))]
    let (model_gateway, llm_cost_table, llm_reload) =
        build_production_model_gateway(boot_provider_factory).await?;

    // Resolved cost table is either: the LLM-policy-derived table (real
    // LLM wired), a test override (so tests can drive deterministic
    // prices through stub gateways), or None — in which case the
    // accountant doesn't get built (no spend, no cascade). The test
    // override (when set) wins over the LLM-derived table — the test is
    // being explicit about the prices it wants.
    let llm_cost_table_arc: Option<Arc<dyn ironclaw_loop_host::ModelCostTable>> =
        llm_cost_table.map(|table| Arc::new(table) as Arc<dyn ironclaw_loop_host::ModelCostTable>);
    #[cfg(any(test, feature = "test-support"))]
    let resolved_cost_table = model_cost_table_override.or(llm_cost_table_arc);
    #[cfg(not(any(test, feature = "test-support")))]
    let resolved_cost_table = llm_cost_table_arc;

    // Build the model budget accountant from the resolved cost table plus
    // the local-dev governor. `BudgetEnforcement::Unenforced` — the resolved
    // trusted-laptop boundary — is the explicit exception: it inherits host
    // trust and must not pause on budget gates. Reading the resolved value
    // rather than the deployment profile means a tenant/org ceiling that
    // narrows yolo away also restores enforcement (§4.4).
    // When neither an LLM policy nor a test override supplies a cost table
    // we deliberately skip the accountant — there's no spend to track and
    // the cascade would never fire.
    //
    // The accountant is wired with a seeding policy derived from the
    // caller-supplied `BudgetDefaults` (or `compiled_defaults().with_env()`
    // as the composition-root fallback when no caller pre-resolves them)
    // so a fresh user / project account picks up the default daily cap on
    // the first model call. Without this seeding step the local-dev
    // governor starts empty and `reserve_with_outcome_in_state` skips
    // accounts that have no configured limit — model calls would record
    // usage but never enforce a cap (review feedback High #2 + Thermo-
    // Nuclear #1: defaults resolve once at the composition root with
    // explicit precedence and a `validate()` call instead of being
    // re-read by the wiring helper).
    let model_budget_accountant: Option<
        Arc<dyn ironclaw_turns::run_profile::LoopModelBudgetAccountant>,
    > = match (
        ironclaw_runtime_policy::budget_enforcement(&runtime_policy),
        resolved_cost_table,
    ) {
        (ironclaw_runtime_policy::BudgetEnforcement::Unenforced, _) => None,
        (_, Some(cost_table)) => {
            let resolved_budget_defaults = match budget_defaults {
                Some(defaults) => {
                    defaults
                        .validate()
                        .map_err(|error| RebornRuntimeError::InvalidArgument {
                            reason: format!("supplied budget defaults invalid: {error}"),
                        })?;
                    defaults
                }
                None => {
                    let defaults = ironclaw_reborn_config::BudgetDefaults::compiled_defaults()
                        .with_env()
                        .map_err(|error| RebornRuntimeError::InvalidArgument {
                            reason: format!("budget defaults env-override invalid: {error}"),
                        })?;
                    defaults
                        .validate()
                        .map_err(|error| RebornRuntimeError::InvalidArgument {
                            reason: format!("resolved budget defaults invalid: {error}"),
                        })?;
                    defaults
                }
            };
            // Shared helper — same wiring shape used by any production
            // loop composer that wants the accountant.
            // The accountant uses the same broadcast-backed sink that
            // the governor writes to, so `BudgetEvent::GateOpened`
            // (emitted by the accountant) lands on the same downstream
            // projection as the governor's `Warned` / `Denied` events.
            let event_sink: Arc<dyn ironclaw_resources::BudgetEventSink> =
                Arc::clone(&broadcast_budget_event_sink)
                    as Arc<dyn ironclaw_resources::BudgetEventSink>;
            let accountant = crate::build_default_budget_accountant(
                Arc::clone(&resource_governor),
                cost_table,
                Arc::clone(&budget_gate_store),
                event_sink,
                &resolved_budget_defaults,
            );
            Some(accountant)
        }
        (_, None) => None,
    };

    let await_dependent_run_evidence: Arc<dyn AwaitDependentRunEvidenceStore> =
        Arc::clone(&subagent_await_edge_evidence);
    let mut loop_exit_evidence = ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
        Arc::clone(&thread_service),
        Arc::clone(&turn_state_store) as Arc<dyn ironclaw_turns::TurnStateStore>,
        Arc::clone(&loop_checkpoint_store) as Arc<dyn ironclaw_turns::LoopCheckpointStore>,
        await_dependent_run_evidence,
        thread_scope.clone(),
    )
    .with_checkpoint_state_store(
        Arc::clone(&checkpoint_state_store) as Arc<dyn ironclaw_turns::CheckpointStateStore>
    );
    if let Some(local_runtime) = local_runtime {
        loop_exit_evidence =
            loop_exit_evidence.with_approval_gate_evidence(Arc::new(ApprovalRequestGateEvidence {
                approval_requests: Arc::clone(&local_runtime.approval_requests)
                    as Arc<dyn ironclaw_run_state::ApprovalRequestStore>,
            }));
        loop_exit_evidence = loop_exit_evidence.with_resource_gate_evidence(
            crate::observability::budget_evidence::budget_gate_evidence(Arc::clone(
                &local_runtime.budget_gate_store,
            )),
        );
    }
    let loop_exit_evidence = Arc::new(loop_exit_evidence);
    let milestone_thread_scope = ThreadScope {
        owner_user_id: Some(actor_user_id.clone()),
        ..thread_scope.clone()
    };
    let milestone_scope = DurableLoopHostMilestoneScope::from_thread_scope(&milestone_thread_scope)
        .map_err(|error| RebornRuntimeError::InvalidArgument {
            reason: error.to_string(),
        })?;
    let durable_milestone_sink: Arc<dyn LoopHostMilestoneSink> = Arc::new(
        DurableLoopHostMilestoneSink::new(Arc::clone(&event_log), milestone_scope),
    );
    if trusted_laptop_access {
        append_trusted_laptop_access_audit(&audit_log, &thread_scope, &actor_user_id).await?;
    }
    let mut projection_services = build_reborn_projection_services(
        Arc::clone(&event_log),
        validated_identity.reply_target_binding_ref.clone(),
    );
    if let Some(local_runtime) = local_runtime {
        projection_services = projection_services
            .with_approval_requests(Arc::clone(&local_runtime.approval_requests)
                as Arc<dyn ironclaw_run_state::ApprovalRequestStore>);
    }
    let live_projection_publisher =
        projection_services.live_projection_publisher(actor_user_id.clone());
    if let Some(skill_activation_source) = &skill_activation_source {
        skill_activation_source
            .set_activation_observer(
                projection_services
                    .skill_activation_observer(Arc::clone(&live_projection_publisher)),
            )
            .map_err(|error| RebornRuntimeError::SkillExecution(error.to_string()))?;
    }
    // The registry is created with the local-runtime services (one instance
    // per runtime) so the trigger-create hook validates per-trigger delivery
    // targets against the same registry product hosts register into.
    let outbound_delivery_target_registry =
        local_runtime.map(|local_runtime| Arc::clone(&local_runtime.outbound_delivery_targets));
    let outbound_preferences_facade: Option<Arc<dyn OutboundPreferencesProductFacade>> =
        match (local_runtime, &outbound_delivery_target_registry) {
            (Some(local_runtime), Some(registry)) => {
                let registry = Arc::clone(registry);
                let provider: Arc<dyn OutboundDeliveryTargetProvider> = registry;
                Some(Arc::new(RebornOutboundPreferencesFacade::new(
                    Arc::clone(&local_runtime.outbound_preferences),
                    provider,
                ))
                    as Arc<dyn OutboundPreferencesProductFacade>)
            }
            _ => None,
        };
    // Clone the live projection publisher for the skill-learning sink before
    // the milestone-sink builder consumes the original by value.
    let skill_learning_publisher = Arc::clone(&live_projection_publisher);
    let milestone_sink = projection_services.with_live_progress_milestone_sink_for_publisher(
        durable_milestone_sink,
        live_projection_publisher,
    );
    let (
        capability_factory,
        capability_input_resolver,
        capability_result_writer,
        capability_surface_resolver,
        model_gateway,
        builtin_capability_policy,
        display_previews,
    ) = if local_runtime.is_some() {
        let builtin_capability_policy = Arc::new(builtin_capability_policy().map_err(|error| {
            tracing::error!(%error, "local-dev capability policy is invalid");
            RebornRuntimeError::InvalidArgument {
                reason: format!("local-dev capability policy is invalid: {error}"),
            }
        })?);
        let local_dev_capabilities = local_dev::capability_wiring(
            &services,
            Arc::clone(&thread_service) as Arc<dyn SessionThreadService>,
            actor_user_id.clone(),
            Arc::clone(&builtin_capability_policy),
            model_gateway,
            milestone_sink.clone(),
            skill_activation_source.clone(),
            outbound_preferences_facade.clone(),
            trajectory_observer,
        )
        .ok_or(RebornRuntimeError::HostRuntimeUnavailable)?;
        (
            local_dev_capabilities.capability_factory,
            local_dev_capabilities.capability_input_resolver,
            local_dev_capabilities.capability_result_writer,
            Arc::new(AllowAllCapabilitySurfaceResolver)
                as Arc<dyn CapabilitySurfaceProfileResolver>,
            local_dev_capabilities.model_gateway,
            Some(builtin_capability_policy),
            Some(local_dev_capabilities.display_previews),
        )
    } else {
        // The trajectory observer is wired only through the local-dev capability
        // path; non-local-dev runtimes have no capability/result hook to forward
        // to. Accepting one here would silently produce an empty trajectory, so
        // fail fast — the seam is local-dev/bench-only (see
        // `RebornRuntimeInput::with_trajectory_observer`).
        if trajectory_observer.is_some() {
            return Err(RebornRuntimeError::InvalidArgument {
                reason: "a trajectory observer was supplied, but it is only supported on \
                         local-dev runtimes; this profile has no local runtime to observe"
                    .to_string(),
            });
        }
        let capability_io = Arc::new(UnavailableCapabilityIo);
        let capability_input_resolver: Arc<dyn LoopCapabilityInputResolver> = capability_io.clone();
        let capability_result_writer: Arc<dyn LoopCapabilityResultWriter> = capability_io;
        let capability_factory: Arc<dyn LoopCapabilityPortFactory> =
            Arc::new(UnavailableCapabilityPortFactory);
        (
            capability_factory,
            capability_input_resolver,
            capability_result_writer,
            Arc::new(EmptyCapabilitySurfaceResolver) as Arc<dyn CapabilitySurfaceProfileResolver>,
            model_gateway,
            None,
            None,
        )
    };
    // Hook framework activation (#3934 + third-party projection), gated behind
    // the typed `HooksActivationConfig` carried in `RebornRuntimeInput` (master
    // flag default OFF; third-party sub-flag also default OFF). The env vars
    // (`HOOKS_ENABLED`, `HOOKS_THIRD_PARTY_ENABLED`) are resolved ONCE at the
    // edge that builds the input (the CLI / ingress adapter); this composition
    // root consumes the typed config and never reads the environment itself.
    //
    // Hook-only projection containment: third-party `[[hooks]]` are discovered
    // and projected into a `HookProjectionRegistry` that carries ONLY hook
    // metadata (no `ExtensionRegistry`, no `ExtensionPackage`) and reaches ONLY
    // this hook factory, not the capability catalog or surface resolver.
    let hook_dispatcher_builder_factory = if let Some(local_runtime) = local_runtime {
        let third_party_input = crate::observability::hooks::ThirdPartyDiscoveryInput {
            filesystem: local_runtime.extension_filesystem.as_ref(),
            tenant_id: &validated_identity.tenant_id,
        };
        let projection_registry = crate::observability::hooks::build_hook_projection_registry(
            builtin_extension_registry()?,
            Some(third_party_input),
            hooks_config,
        )
        .await
        .map_err(|error| RebornRuntimeError::InvalidArgument {
            reason: format!("hook projection registry assembly failed: {error}"),
        })?;
        crate::observability::hooks::build_hook_dispatcher_builder_factory_for_tenant(
            hooks_config,
            &projection_registry,
            &validated_identity.tenant_id,
        )
        .map_err(|error| RebornRuntimeError::InvalidArgument {
            reason: format!("hook framework activation failed: {error}"),
        })?
    } else if hooks_config.is_enabled() {
        return Err(RebornRuntimeError::MalformedConfig {
            reason: "hook framework is not supported or wired for production runtime launch"
                .to_string(),
        });
    } else {
        None
    };

    // Autonomous Trace Commons capture: a best-effort lifecycle sink mirrors
    // the v1 binary's turn-end capture. Policy-gated per user scope — the
    // sink is inert (one policy-file read per turn) until a scope enrolls
    // via `builtin.trace_commons.onboard` or `traces opt-in`.
    // Seed with the runtime owner's TENANT-SCOPED key (matching how capture
    // keys state), so startup pending-queue discovery finds the owner's queued
    // traces — a bare owner id would miss the `trace_scope_key(tenant, owner)`
    // queue dir.
    let runtime_owner_trace_scope = ironclaw_reborn_traces::contribution::trace_scope_key(
        thread_scope.tenant_id.as_str(),
        actor_user_id.as_str(),
    );
    let trace_capture_scopes: crate::observability::trace_capture::ObservedTraceScopes =
        Arc::new(std::sync::Mutex::new(std::collections::BTreeSet::from([
            runtime_owner_trace_scope,
        ])));
    let trace_capture_sink: Arc<dyn ironclaw_turns::TurnEventSink> = Arc::new(
        crate::observability::trace_capture::TraceCaptureTurnEventSink::new(
            Arc::clone(&thread_service),
            Arc::clone(&trace_capture_scopes),
        ),
    );
    let projection_turn_event_wake_sink = projection_services.turn_event_wake_sink();
    // Skill learning shares the turn-end seam with trace capture (composed
    // additively, so the trace-capture path is unchanged). It is active only
    // when a learning model is configured (a stronger model than the run's, via
    // IRONCLAW_SKILL_LEARNING_MODEL); otherwise only trace capture runs.
    let mut turn_event_sinks: Vec<Arc<dyn ironclaw_turns::TurnEventSink>> =
        vec![trace_capture_sink, projection_turn_event_wake_sink];
    let mut skill_learning_extraction_tasks: Option<
        Arc<crate::extension_host::skill_learning::SkillLearningExtractionTasks>,
    > = None;
    if let (Some((learning_provider, learning_model)), Some(local_runtime)) =
        (skill_learning_provider, local_runtime)
    {
        let inference: Arc<dyn ironclaw_skills::learning::SkillInferencePort> = Arc::new(
            crate::extension_host::skill_learning::SkillLearningInferenceAdapter::new(
                learning_provider,
                learning_model,
            ),
        );
        // Reuse the runtime's already-built scoped skill-management port so the
        // learned skill lands exactly where the WebUI lists it and the next run
        // loads it. The writer evolves an existing learned skill in place when a
        // recurring task is re-learned, using the same learning model to refine
        // it (accumulated gotchas, bumped version) instead of accreting siblings.
        let skill_refiner: Arc<dyn crate::extension_host::skill_learning::SkillRefiner> = Arc::new(
            crate::extension_host::skill_learning::LlmSkillRefiner::new(Arc::clone(&inference)),
        );
        let skill_writer: Arc<dyn crate::extension_host::skill_learning::SkillWriter> =
            Arc::new(crate::extension_host::skill_learning::PortSkillWriter::new(
                Arc::clone(&local_runtime.skill_management),
                skill_refiner,
            ));
        // Live "learned a skill" bubble on the run's thread stream (reuses the
        // SkillActivation projection -> existing chat bubble).
        let skill_learned_notifier: Arc<
            dyn crate::extension_host::skill_learning::SkillLearnedNotifier,
        > = Arc::new(
            crate::extension_host::skill_learning::LiveSkillLearnedNotifier::new(
                skill_learning_publisher,
            ),
        );
        let extraction_tasks =
            Arc::new(crate::extension_host::skill_learning::SkillLearningExtractionTasks::new());
        skill_learning_extraction_tasks = Some(Arc::clone(&extraction_tasks));
        turn_event_sinks.push(Arc::new(
            crate::extension_host::skill_learning::SkillLearningTurnEventSink::new(
                Arc::clone(&thread_service),
                inference,
                skill_writer,
                skill_learned_notifier,
                extraction_tasks,
            ),
        ));
    }
    let turn_event_sink: Arc<dyn ironclaw_turns::TurnEventSink> = Arc::new(
        crate::extension_host::skill_learning::CompositeTurnEventSink::new(turn_event_sinks),
    );

    let communication_context_provider: Option<
        Arc<dyn ironclaw_turns::run_profile::CommunicationContextProvider>,
    > = match (local_runtime, outbound_preferences_facade.clone()) {
        (Some(local_runtime), Some(outbound_preferences_facade)) => {
            let mut lifecycle_facade =
                crate::extension_host::lifecycle::RebornLocalLifecycleFacade::new(Arc::clone(
                    &local_runtime.skill_management,
                ));
            if let Some(extension_management) = &local_runtime.extension_management {
                lifecycle_facade =
                    lifecycle_facade.with_extension_management(Arc::clone(extension_management));
            }
            Some(Arc::new(
                crate::root::communication_context::RuntimeCommunicationContextProvider::new(
                    outbound_preferences_facade,
                )
                .with_lifecycle_facade(Arc::new(lifecycle_facade)),
            )
                as Arc<
                    dyn ironclaw_turns::run_profile::CommunicationContextProvider,
                >)
        }
        _ => None,
    };

    // Resolve the disclosure mode once so the runtime config and the system-prompt
    // disclosure-protocol injection agree on a single value.
    let resolved_tool_disclosure = tool_disclosure.unwrap_or_else(ToolDisclosureMode::from_env);
    let default_runtime_config = DefaultPlannedRuntimeConfig::default();

    // Deferred bind (§ await-edge resolver ordering note above,
    // `RuntimeStoreParts`'s doc comment): the resolver was assembled inside
    // `local_runtime_parts`/`production_runtime_parts` before
    // `capability_result_writer` existed. Bind it now, exactly once, before
    // the resolver's settler ever runs.
    subagent_await_edge_settler
        .bind_result_writer(Arc::clone(&capability_result_writer))
        .map_err(|error| RebornRuntimeError::MalformedConfig {
            reason: format!("await-edge resolver result writer bind failed: {error}"),
        })?;

    let planned_runtime_parts = DefaultPlannedRuntimeParts {
        turn_state: Arc::clone(&turn_state_store),
        thread_service: Arc::clone(&thread_service),
        thread_scope: thread_scope.clone(),
        // Read landed attachment bytes back through the project workspace
        // filesystem so the model port can build multimodal image parts for
        // vision-capable models. Only available when a local runtime (and thus a
        // workspace filesystem) is composed.
        attachment_read_port: local_runtime.map(|rt| {
            Arc::new(crate::support::fs::ProjectScopedAttachmentReader::new(
                Arc::clone(&rt.workspace_filesystem),
            )) as Arc<dyn ironclaw_loop_host::LoopAttachmentReadPort>
        }),
        // §5.2.9 render-from-record: a `FilesystemGateRecordStore` over the SAME
        // shared `extension_filesystem` + per-user mount view the local-dev
        // capability port persists `GateRecord::Auth` into (see
        // `runtime/local_dev.rs`'s `wire_local_dev_capability_port`, which builds
        // its store the same way and passes it via `with_gate_record_store`).
        // Both are stateless views over one durable Arc, so the turn executor
        // reads back exactly the record the capability port saved under the
        // matching owner scope. The two constructions MUST stay over the same
        // filesystem/scope.
        gate_record_store: local_runtime.map(|rt| {
            Arc::new(ironclaw_run_state::FilesystemGateRecordStore::new(
                crate::wrap_scoped(Arc::clone(&rt.extension_filesystem)),
            )) as Arc<dyn ironclaw_run_state::GateRecordStore>
        }),
        model_gateway: Arc::clone(&model_gateway),
        checkpoint_state_store: Arc::clone(&checkpoint_state_store)
            as Arc<dyn ironclaw_turns::CheckpointStateStore>,
        loop_checkpoint_store: Arc::clone(&loop_checkpoint_store)
            as Arc<dyn ironclaw_turns::LoopCheckpointStore>,
        milestone_sink,
        capability_factory,
        capability_surface_resolver,
        capability_result_writer,
        subagent_goal_store,
        subagent_await_edge_writer,
        subagent_await_edge_settler,
        subagent_await_edge_evidence,
        subagent_definition_resolver: Arc::new(StaticSubagentDefinitionResolver),
        subagent_spawn_input_codec: Arc::new(JsonSpawnSubagentInputCodec::new(
            capability_input_resolver,
        )),
        subagent_spawn_limits: ironclaw_loop_host::SubagentSpawnLimits::default(),
        loop_exit_evidence,
        config: DefaultPlannedRuntimeConfig {
            heartbeat_interval: runner.heartbeat_interval,
            poll_interval: runner.poll_interval,
            lease_recovery_interval: default_runtime_config.lease_recovery_interval,
            worker_count: runner.worker_count,
            disabled_capability_ids: default_runtime_config.disabled_capability_ids,
            text_only_driver: Default::default(),
            host: Default::default(),
            tool_disclosure: resolved_tool_disclosure,
            planned_default_iteration_limit: optional_nonzero_u32_env(
                "IRONCLAW_REBORN_PLANNED_DEFAULT_ITERATION_LIMIT",
            )?,
            planned_model_availability_retry_attempts: {
                #[cfg(any(test, feature = "test-support"))]
                let resolved = match model_availability_retry_attempts_override {
                    Some(attempts) => Some(attempts),
                    None => optional_nonzero_u32_env(
                        "IRONCLAW_REBORN_MODEL_AVAILABILITY_RETRY_ATTEMPTS",
                    )?,
                };
                #[cfg(not(any(test, feature = "test-support")))]
                let resolved =
                    optional_nonzero_u32_env("IRONCLAW_REBORN_MODEL_AVAILABILITY_RETRY_ATTEMPTS")?;
                resolved
            },
        },
        model_route_resolver: None,
        cancellation_factory: None,
        skill_context_source,
        input_queue: None,
        identity_context_source: match local_runtime {
            Some(local_runtime) => Arc::new(
                // Local-dev seeding validates the prompt path first, so non-file prompt paths fail
                // as build errors before this runtime-level identity-source guard is reached.
                DefaultSystemPromptIdentitySource::try_new(
                    local_runtime.local_dev_storage_root.clone(),
                    local_runtime.default_system_prompt_path.clone(),
                    resolved_tool_disclosure.is_bridged(),
                )
                .map_err(|error| RebornRuntimeError::InvalidArgument {
                    reason: error.to_string(),
                })?,
            ) as Arc<dyn HostIdentityContextSource>,
            None => Arc::new(EmptyIdentityContextSource) as Arc<dyn HostIdentityContextSource>,
        },
        // Resolve the per-user agent-context profile (timezone/locale/location) from
        // `context/profile.json` via the workspace filesystem. When a local-dev workspace
        // filesystem is available, the `MemoryBackedUserProfileSource` adapter reads it;
        // otherwise `EmptyUserProfileSource` degrades gracefully to `None` (profile unknown).
        // `extension_filesystem` is the raw `Arc<CompositeRootFilesystem>` (=
        // `CompositeRootFilesystem`) — the underlying RootFilesystem the workspace
        // mounts are built from. `MemoryBackedUserProfileSource` constructs its own
        // full virtual paths via `profile_scope_and_path` and does not use the
        // `ScopedFilesystem` mount view, so the raw `RootFilesystem` is correct here.
        //
        // NOTE: this `Some(local_runtime) => real / None => Empty` guard intentionally
        // mirrors `identity_context_source` directly above. The production-graph path
        // (`production_runtime_parts`, `local_runtime: None`) currently wires NEITHER the
        // identity source NOR this profile source — both degrade to Empty there today.
        // Wiring the production-graph composition for these optional context sources is a
        // single deferred follow-up (identity + profile together, to keep them paired);
        // do not wire only one of them here, or they will diverge. See issue #5013.
        user_profile_source: match local_runtime {
            Some(local_runtime) => Arc::new(MemoryBackedUserProfileSourceAdapter(
                MemoryBackedUserProfileSource::new(Arc::clone(&local_runtime.extension_filesystem)
                    as Arc<dyn ironclaw_filesystem::RootFilesystem>),
            )) as Arc<dyn HostUserProfileSource>,
            None => Arc::new(EmptyUserProfileSource) as Arc<dyn HostUserProfileSource>,
        },
        model_policy_guard: None,
        model_budget_accountant,
        safety_context: None,
        hook_security_audit_sink: Some(Arc::new(ironclaw_events::TracingSecurityAuditSink)),
        turn_event_sink: Some(turn_event_sink),
        hook_dispatcher_builder_factory,
        communication_context_provider,
        // For the production composition path, use the pre-minted wiring from
        // `build_production_shaped` so the `HostRuntimeServices` notifier (used by
        // `turn_coordinator_for_production`) and the scheduler's wake loop share the
        // exact same channel. For local-dev, `None` causes `build_default_planned_runtime`
        // to mint its own wiring internally (existing behavior).
        scheduler_wake_wiring: production_scheduler_wake,
    };
    let composition = build_default_planned_runtime(planned_runtime_parts)?;
    let default_resolved_run_profile = composition
        .run_profile_resolver
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .map_err(|error| RebornRuntimeError::InvalidArgument {
            reason: format!("could not resolve default run profile: {error}"),
        })?;
    let default_run_profile_id = default_resolved_run_profile.profile_id.as_str().to_string();
    let failure_explanation_thread_id =
        ThreadId::new("failure-explanation-system").map_err(|reason| {
            RebornRuntimeError::InvalidArgument {
                reason: format!("failure explanation thread id: {reason}"),
            }
        })?;
    let failure_explanation_scope = TurnScope::new(
        thread_scope.tenant_id.clone(),
        Some(thread_scope.agent_id.clone()),
        thread_scope.project_id.clone(),
        failure_explanation_thread_id,
    );
    let failure_explanation_profile = default_resolved_run_profile.clone();
    let failure_explanation_model_gateway = Arc::clone(&model_gateway);
    let failure_explanation_inference = Arc::new(move || {
        Arc::new(ModelGatewayBackedSystemInferencePort::new(
            Arc::clone(&failure_explanation_model_gateway),
            LoopRunContext::new(
                failure_explanation_scope.clone(),
                TurnId::new(),
                TurnRunId::new(),
                failure_explanation_profile.clone(),
            ),
        )) as Arc<dyn ironclaw_turns::run_profile::SystemInferencePort>
    });
    let planned_turn_coordinator: Arc<dyn TurnCoordinator> = composition.coordinator.clone();
    let approval_audit_sink = Arc::new(InMemoryAuditSink::new());
    let approval_interaction_service: Arc<dyn ApprovalInteractionService> =
        if let (Some(local_runtime), Some(builtin_capability_policy)) =
            (local_runtime, builtin_capability_policy)
        {
            build_approval_interaction_service(
                local_runtime,
                builtin_capability_policy,
                Arc::clone(&planned_turn_coordinator),
                Some(approval_audit_sink.clone()),
            )?
        } else {
            Arc::new(UnavailableApprovalInteractionService)
        };
    let auth_interaction_service = if let Some(local_runtime) = local_runtime {
        build_webui_auth_interaction_service(
            services.product_auth.as_deref(),
            Arc::clone(&local_runtime.turn_state),
            Arc::clone(&planned_turn_coordinator),
        )
    } else {
        Arc::new(auth_interaction::UnavailableAuthInteractionService)
    };
    let turn_event_source: Arc<dyn TurnEventProjectionSource> = turn_state_store.clone();
    let mut projection_services = projection_services
        .with_turn_events(turn_event_source, Arc::clone(&planned_turn_coordinator))
        .with_model_failure_explainer_factory(failure_explanation_inference);
    if let Some(display_previews) = display_previews {
        projection_services = projection_services.with_display_previews(display_previews);
    }
    // Wire auth-challenge enrichment when the product-auth bundle exposes a
    // flow record source (local-dev / test mode). Production deployments without
    // a wired flow_record_source fall back to the plain 4-field AuthPromptView.
    let projection_services = if let Some(provider) = services
        .product_auth
        .as_ref()
        .and_then(|pa| pa.as_auth_challenge_provider())
    {
        projection_services.with_auth_challenges(provider)
    } else {
        projection_services
    };
    services.turn_coordinator = Some(Arc::clone(&planned_turn_coordinator));

    // Generic channel host assembly (extension-runtime P6 S2): reconcile
    // per-extension inbound-channel registrations from the generic host's
    // active snapshot for EVERY composed runtime with a generic host. The
    // run-delivery observer half follows the delivery coordinator's
    // availability (no coordinator -> ingress-only registrations).
    let channel_host_assembly = {
        let approval_context = services.local_runtime.as_ref().map(|local_runtime| {
            Arc::new(
                crate::extension_host::run_delivery_ports::ProjectionApprovalPromptContextSource::new(
                    local_runtime.approval_requests.clone()
                        as Arc<dyn ironclaw_run_state::ApprovalRequestStore>,
                ),
            ) as Arc<dyn ironclaw_product_workflow::ApprovalPromptContextSource>
        });
        let blocked_auth_prompts = Some(Arc::new(
            crate::extension_host::run_delivery_ports::ProductAuthBlockedAuthPromptSource::new(
                services
                    .product_auth
                    .as_ref()
                    .and_then(|product_auth| product_auth.as_auth_challenge_provider()),
            ),
        )
            as Arc<dyn ironclaw_product_workflow::BlockedAuthPromptSource>);
        let auth_flow_cancel = services
            .product_auth
            .as_ref()
            .and_then(|product_auth| product_auth.as_blocked_auth_flow_canceller());
        services.start_channel_host_assembly(crate::factory::ChannelHostAssemblyWiring {
            thread_service: Arc::clone(&thread_service),
            turn_coordinator: Arc::clone(&planned_turn_coordinator),
            approval_interaction: Some(Arc::clone(&approval_interaction_service)),
            auth_interaction: Some(Arc::clone(&auth_interaction_service)),
            identity: crate::extension_host::channel_host::ChannelHostIdentity {
                tenant_id: thread_scope.tenant_id.clone(),
                agent_id: thread_scope.agent_id.clone(),
                project_id: thread_scope.project_id.clone(),
                operator_user_id: actor_user_id.clone(),
            },
            approval_context,
            blocked_auth_prompts,
            auth_flow_cancel,
            run_delivery_settings: ironclaw_product_workflow::RunDeliverySettings::default(),
        })
    };

    // The binary-assembled channel-extension extras (extension-runtime
    // DEL-7): gate-reply classifiers + preference-target codecs registered
    // on the assembly for every supplied channel binding.
    if let Some(assembly) = channel_host_assembly.as_ref() {
        for binding in &services.channel_extension_bindings {
            assembly
                .register_extras(
                    &binding.extension_id,
                    crate::extension_host::channel_host::ChannelExtras {
                        classifier: binding.inbound_payload_classifier.clone(),
                        preference_target_codec: binding.preference_target_codec.clone(),
                        subject_route_resolver: None,
                        storage_roots: None,
                    },
                )
                .await;
        }
    }

    // Generic outbound-delivery targets (extension-runtime P6): one provider
    // over the assembly's vendor codecs, the `[channel.config]` routing
    // values, and the generic DM-target store serves every active channel
    // extension.
    if let (Some(registry), Some(assembly), Some(local_runtime)) = (
        outbound_delivery_target_registry.as_ref(),
        channel_host_assembly.as_ref(),
        local_runtime,
    ) && let (Some(channel_config), Some(dm_targets)) = (
        local_runtime.channel_config.clone(),
        local_runtime.channel_dm_target_store.clone(),
    ) {
        crate::extension_host::channel_outbound_targets::register_generic_channel_outbound_targets(
            registry,
            crate::extension_host::channel_outbound_targets::GenericChannelOutboundTargetDeps {
                watch: assembly.snapshot_watch(),
                assembly: Arc::clone(assembly),
                channel_config,
                dm_targets,
                identity:
                    crate::extension_host::channel_outbound_targets::ChannelOutboundTargetIdentity {
                        tenant_id: thread_scope.tenant_id.clone(),
                        agent_id: thread_scope.agent_id.clone(),
                        project_id: thread_scope.project_id.clone(),
                    },
            },
        );
    }

    // `trigger_poller_handle`, `post_submit_hook_slot`, and the test-support
    // `trigger_conversation_pairing_value` are produced atomically inside
    // a single `if trigger_poller.enabled` expression. Avoid a
    // `let mut … = None` sentinel pattern flagged by code review
    // (review f-ptr-3): the `let X;` deferred-init form is single-assign
    // per branch and Rust's borrow checker prevents reads before init.
    let trigger_poller_handle: Option<TriggerPollerRuntimeHandle>;
    let runtime_post_submit_hook_slot: Option<
        Arc<
            std::sync::OnceLock<Arc<dyn crate::automation::trigger_poller::PostSubmitDeliveryHook>>,
        >,
    >;
    #[cfg(any(test, feature = "test-support"))]
    let trigger_conversation_pairing_value: Option<
        Arc<dyn ironclaw_conversations::ConversationActorPairingService>,
    >;
    if trigger_poller.enabled {
        // Fire-time authorizer: an explicit override wins (tests/advanced),
        // otherwise build one from the deployment's `TriggerFireAccessPolicy`
        // (arch-simplification §4.4 — the former `local_trigger_access` store is
        // now a config value, not a per-deployment store type). Grants are
        // OR-combined, preserving the union the single store expressed.
        let mut grant_checkers: Vec<Arc<dyn TriggerFireAccessChecker>> = Vec::new();
        for grant in trigger_fire_access.grants() {
            match grant {
                TriggerFireAccessGrant::StaticOwner {
                    owner,
                    agent,
                    project,
                } => {
                    let checker: Arc<dyn TriggerFireAccessChecker> =
                        Arc::new(StaticOwnerTriggerFireChecker::new(
                            thread_scope.tenant_id.clone(),
                            owner.clone(),
                            agent.clone(),
                            project.clone(),
                        ));
                    grant_checkers.push(checker);
                }
                TriggerFireAccessGrant::TenantMembership { agent, project } => {
                    // Membership is resolved against the canonical identity
                    // directory the SSO login path populates — the same store
                    // `reborn_user_directory` opens, built here from whichever
                    // substrate (local or production-shaped) backs this runtime.
                    // Fail closed if no substrate can provide the directory.
                    let directory = poller_user_directory(
                        &services,
                        local_runtime,
                        &thread_scope.tenant_id,
                        &actor_user_id,
                        &thread_scope.agent_id,
                        thread_scope.project_id.as_ref(),
                    )
                    .ok_or(RebornRuntimeError::InvalidArgument {
                        reason: "trigger poller TenantMembership grant requires an identity \
                                 directory, but no runtime substrate is present"
                            .to_string(),
                    })?;
                    let checker: Arc<dyn TriggerFireAccessChecker> =
                        Arc::new(IdentityMembershipTriggerFireChecker::new(
                            directory,
                            thread_scope.tenant_id.clone(),
                            agent.clone(),
                            project.clone(),
                        ));
                    grant_checkers.push(checker);
                }
            }
        }
        let policy_checker: Option<Arc<dyn TriggerFireAccessChecker>> = if grant_checkers.len() <= 1
        {
            grant_checkers.into_iter().next()
        } else {
            let composite: Arc<dyn TriggerFireAccessChecker> =
                Arc::new(CompositeTriggerFireChecker::new(grant_checkers));
            Some(composite)
        };
        let effective_trigger_fire_access_checker =
            trigger_fire_access_checker.clone().or(policy_checker);
        validate_trigger_poller_authorization(
            &trigger_poller,
            effective_trigger_fire_access_checker.as_ref(),
        )?;
        // Resolve the trigger conversation services from whichever substrate
        // backs this runtime — the local substrate's (lazily-built durable /
        // in-memory) services, or the production store graph's eagerly-built
        // filesystem services. The poller launch is otherwise identical for
        // both, so there is a single substrate-agnostic path below.
        let conversation_services = match local_runtime {
            Some(local) => local
                .durable_trigger_conversation_services()
                .await
                .map_err(|error| RebornRuntimeError::InvalidArgument {
                    reason: format!("trigger conversation services unavailable: {error}"),
                })?,
            None => match services.production_runtime.as_ref() {
                Some(crate::factory::RebornProductionRuntimeServices::LibSql(graph)) => {
                    graph.trigger_conversation_services.clone()
                }
                Some(crate::factory::RebornProductionRuntimeServices::Postgres(graph)) => {
                    graph.trigger_conversation_services.clone()
                }
                None => {
                    return Err(RebornRuntimeError::InvalidArgument {
                        reason: "trigger poller enabled but no runtime substrate present"
                            .to_string(),
                    });
                }
            },
        };
        let trigger_poller_services = build_trigger_poller_services(
            conversation_services,
            Arc::clone(&planned_turn_coordinator),
            Arc::clone(&thread_service),
            trigger_poller.authorizer,
            effective_trigger_fire_access_checker.clone(),
            thread_scope.tenant_id.clone(),
            validated_identity.agent_id.clone(),
        )?;
        let active_run_lookup =
            build_trigger_active_run_lookup(Arc::clone(&turn_run_snapshot_source));
        // Fail closed if the substrate produced no trigger repository (both the
        // local and production parts always set `Some`; a `None` here means the
        // runtime was assembled without a substrate, which the poller cannot
        // service).
        let trigger_repository =
            trigger_repository
                .clone()
                .ok_or(RebornRuntimeError::InvalidArgument {
                    reason: "trigger poller enabled but no trigger repository present".to_string(),
                })?;
        #[cfg(any(test, feature = "test-support"))]
        {
            trigger_conversation_pairing_value =
                Some(Arc::clone(&trigger_poller_services.pairing_service));
        }
        let hook_slot = Arc::clone(&trigger_poller_services.post_submit_hook_slot);
        runtime_post_submit_hook_slot = Some(Arc::clone(&hook_slot));
        trigger_poller_handle = spawn_trigger_poller(
            trigger_poller,
            TriggerPollerCompositionDeps {
                repository: trigger_repository,
                materializer: trigger_poller_services.materializer,
                trusted_submitter: trigger_poller_services.trusted_submitter,
                active_run_lookup,
                post_submit_hook_slot: hook_slot,
            },
        )
        .map_err(|error| RebornRuntimeError::InvalidArgument {
            reason: format!("trigger poller could not be started: {error}"),
        })?;
    } else {
        trigger_poller_handle = None;
        runtime_post_submit_hook_slot = None;
        #[cfg(any(test, feature = "test-support"))]
        {
            trigger_conversation_pairing_value = None;
        }
    }

    // Generic triggered-run delivery (extension-runtime P6): one hook routes
    // each settled trigger fire to the owning channel extension's driver via
    // the assembly's vendor codecs.
    if let (Some(slot), Some(assembly), Some(local_runtime)) = (
        runtime_post_submit_hook_slot.as_ref(),
        channel_host_assembly.as_ref(),
        local_runtime,
    ) {
        let generic_trigger_hook: Arc<
            dyn crate::automation::trigger_poller::PostSubmitDeliveryHook,
        > = Arc::new(
            crate::extension_host::channel_triggered_delivery::GenericTriggeredRunDeliveryHook::new(
                Arc::clone(assembly),
                Arc::clone(&local_runtime.triggered_run_delivery),
                Arc::clone(&local_runtime.outbound_preferences),
                Arc::clone(&local_runtime.outbound_delivery_targets),
            ),
        );
        if slot.set(generic_trigger_hook).is_err() {
            tracing::debug!(
                "generic triggered-run delivery hook slot was already occupied; keeping the first hook"
            );
        }
    }

    let scheduler_notifier = composition.scheduler_handle.wake_notifier();

    // Spawn the engine-owned credential keepalive sweep (B4;
    // `ironclaw_auth::keepalive`). The factory reports whether the durable
    // candidate source, recipe data, leader lock, and refresh port are ready
    // together. Local-dev and override paths report `Absent`; the `enabled`
    // policy flag still gates the actual spawn inside `spawn_keepalive_sweep`.
    let credential_refresh_worker_handle = match std::mem::replace(
        &mut services.credential_refresh_worker,
        crate::factory::CredentialRefreshWorkerReady::Absent,
    ) {
        crate::factory::CredentialRefreshWorkerReady::Ready {
            candidate_source,
            recipes,
            leader_lock,
            refresh_port,
        } => ironclaw_auth::spawn_keepalive_sweep(
            credential_refresh,
            ironclaw_auth::KeepaliveSweepDeps {
                candidates: candidate_source,
                recipes,
                refresh: refresh_port as std::sync::Arc<dyn ironclaw_auth::KeepaliveRefreshPort>,
                leader_lock: std::sync::Arc::new(leader_lock),
            },
        ),
        crate::factory::CredentialRefreshWorkerReady::Absent => None,
    };
    let trace_flush_worker =
        crate::observability::trace_capture::spawn_trace_queue_flush_worker(trace_capture_scopes);
    // Scheduler is running (started inside build_default_planned_runtime); mark readiness.
    services.readiness.workers.turn_runner = true;
    services.readiness.workers.trigger_poller = trigger_poller_handle.is_some();
    let turn_coordinator = planned_turn_coordinator;

    // Spawn the budget-event projection task as the production owner
    // of the broadcast sink — review feedback Thermo-Nuclear #3
    // (#3841 follow-up A2). The runtime's `broadcast_budget_event_sink`
    // accessor used to expose a sink that no one subscribed to; with
    // this projection the runtime always has at least the tracing
    // observer attached, and callers can install a richer observer
    // (SSE projection, telemetry export) through
    // `RebornRuntimeInput::with_budget_event_observer`.
    let budget_event_projection = Some({
        let observer = budget_event_observer.unwrap_or_else(|| {
            Arc::new(crate::TracingBudgetEventObserver) as Arc<dyn crate::BudgetEventObserver>
        });
        crate::observability::budget_events::BudgetEventProjection::spawn(
            broadcast_budget_event_sink.as_ref(),
            observer,
        )
    });

    // Row-store handle for the graceful-shutdown write-behind drain (see the
    // field doc). `local_runtime` is `Option<&…>` (`Copy`), so mapping it here
    // doesn't disturb its later use.
    let turn_state_flush = local_runtime.map(|lr| Arc::clone(&lr.turn_state));

    // Apply the effective LLM config (config.toml/env selection + any stored
    // key) to the placeholder gateway exactly once, via the same live-reload
    // path the settings UI uses (see `webui_llm_reload_trigger`). Failure
    // degrades like a boot with no LLM configured: placeholder stays wired,
    // operator retries through Settings -> Inference without a restart.
    if let (Some(boot_config), Some(reload_parts)) = (boot.as_ref(), llm_reload.as_ref()) {
        let boot_reload_adapter = crate::llm_admin::llm_reload::RebornLlmReloadAdapter::new(
            boot_config.clone(),
            Arc::clone(&reload_parts.reload_handle),
            Arc::clone(&reload_parts.session),
            crate::LlmKeyStore::new(services.secret_store()),
        );
        if let Err(error) = crate::LlmReloadTrigger::reload(&boot_reload_adapter).await {
            tracing::warn!(
                %error,
                "boot-time LLM reload failed; the placeholder provider stays active until the \
                 next successful reload (e.g. through Settings -> Inference)"
            );
        }
    }

    let runtime = RebornRuntime {
        services,
        turn_coordinator,
        channel_host_assembly,
        turn_state_flush,
        turn_tree_store: turn_state_store,
        thread_service,
        thread_scope,
        turn_scheduler: RuntimeTurnScheduler::new(composition.scheduler_handle, scheduler_notifier),
        trigger_poller_handle,
        credential_refresh_worker_handle,
        trace_flush_worker,
        skill_learning_extraction_tasks,
        #[cfg(any(test, feature = "test-support"))]
        trigger_conversation_pairing: trigger_conversation_pairing_value,
        outbound_delivery_target_registry,
        budget_event_projection,
        poll_settings: poll,
        admin_api_token_minter,
        actor_user_id,
        source_binding_ref: validated_identity.source_binding_ref,
        reply_target_binding_ref: validated_identity.reply_target_binding_ref,
        projection_services,
        approval_interaction_service,
        auth_interaction_service,
        #[cfg(test)]
        approval_audit_sink,
        webui_event_log: event_log,
        default_run_profile_id,
        send_locks: Mutex::new(HashMap::new()),
        skill_activation_source,
        skill_execution_adapter,
        boot,
        llm_reload,
    };
    // Fill the composition's late-bound channel-connection facade slot (§6.4)
    // now the runtime's serving tenant is known: extension removal
    // (`RebornLocalExtensionManagementPort::remove`) disconnects the caller's
    // channel identity through this facade, and the identity-binding write
    // hook is only reachable from runtime-backed compositions — so filling
    // here keeps "wherever a binding can be written, removal disconnects it".
    // First write wins by `OnceLock` contract: a test bundle that filled the
    // slot before the runtime was built keeps its facade (same stores, same
    // durable state), so the discarded `set` result is deliberate.
    if let Some(local_runtime) = runtime.services.local_runtime.as_ref()
        && let Some(channel_connection) = runtime.generic_channel_connection_facade()
    {
        let _ = local_runtime
            .channel_disconnect_slot
            .set(channel_connection);
    }
    Ok(runtime)
}

/// Thin wrapper over
/// `build_webui_auth_interaction_service_with_turn_run_source` using
/// `turn_state_store` (production always passes `local_runtime.turn_state`)
/// as the turn-run snapshot source — production behavior is unchanged by the
/// seam below.
fn build_webui_auth_interaction_service(
    product_auth: Option<&RebornProductAuthServices>,
    turn_state_store: Arc<ComposedTurnStateStore>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
) -> Arc<dyn AuthInteractionService> {
    build_webui_auth_interaction_service_with_turn_run_source(
        product_auth,
        turn_state_store as Arc<dyn TurnRunSnapshotSource>,
        turn_coordinator,
    )
}

/// Identical to [`build_webui_auth_interaction_service`] except
/// the auth read model reads `turn_run_source` instead of a hardcoded
/// `ComposedTurnStateStore`. See
/// `build_approval_interaction_service_with_turn_run_source`'s doc
/// for why this seam exists.
fn build_webui_auth_interaction_service_with_turn_run_source(
    product_auth: Option<&RebornProductAuthServices>,
    turn_run_source: Arc<dyn TurnRunSnapshotSource>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
) -> Arc<dyn AuthInteractionService> {
    // `AuthFlowRecordSource` is optional on the product-auth bundle because
    // production may supply a durable read projection that is not the flow
    // manager itself. Local-dev can render pending WebUI auth interactions only
    // when the bundle explicitly exposes this scoped projection; otherwise the
    // WebUI surface fails closed with a stable unavailable error.
    let Some(product_auth) = product_auth else {
        return Arc::new(auth_interaction::UnavailableAuthInteractionService);
    };
    let Some(flow_records) = product_auth.flow_record_source() else {
        return Arc::new(auth_interaction::UnavailableAuthInteractionService);
    };
    Arc::new(DefaultAuthInteractionService::new(
        Arc::new(auth_interaction::SnapshotAuthInteractionReadModel::new(
            turn_run_source,
            flow_records,
        )),
        product_auth.flow_manager(),
        turn_coordinator,
    ))
}

const LOOP_RUN_CAPABILITY_ID: &str = "loop.run";
const TRUSTED_LAPTOP_ACCESS_AUDIT_KIND: &str = "local_dev_trusted_laptop_access";
const TRUSTED_LAPTOP_ACCESS_AUDIT_TARGET: &str = "filesystem=host_workspace_and_home;process=local_host;network=direct;secrets=inherited_env;host_home_mount=/host";
const TRUSTED_LAPTOP_ACCESS_AUDIT_STATUS: &str = "host_home_mounted_read_write";

async fn append_trusted_laptop_access_audit(
    audit_log: &Arc<dyn DurableAuditLog>,
    thread_scope: &ThreadScope,
    actor_user_id: &UserId,
) -> Result<(), RebornRuntimeError> {
    let invocation_id = InvocationId::new();
    audit_log
        .append(AuditEnvelope {
            event_id: AuditEventId::new(),
            correlation_id: CorrelationId::new(),
            stage: AuditStage::After,
            timestamp: Utc::now(),
            tenant_id: thread_scope.tenant_id.clone(),
            user_id: actor_user_id.clone(),
            agent_id: Some(thread_scope.agent_id.clone()),
            project_id: thread_scope.project_id.clone(),
            mission_id: thread_scope.mission_id.clone(),
            thread_id: None,
            invocation_id,
            process_id: None,
            approval_request_id: None,
            extension_id: None,
            action: ActionSummary {
                kind: TRUSTED_LAPTOP_ACCESS_AUDIT_KIND.to_string(),
                target: Some(TRUSTED_LAPTOP_ACCESS_AUDIT_TARGET.to_string()),
                effects: vec![
                    EffectKind::ReadFilesystem,
                    EffectKind::WriteFilesystem,
                    EffectKind::SpawnProcess,
                    EffectKind::Network,
                    EffectKind::UseSecret,
                ],
            },
            decision: DecisionSummary {
                kind: "allowed".to_string(),
                reason: None,
                actor: None,
            },
            result: Some(ActionResultSummary {
                success: true,
                status: Some(TRUSTED_LAPTOP_ACCESS_AUDIT_STATUS.to_string()),
                output_bytes: None,
            }),
        })
        .await
        .map(|_| ())
        .map_err(|error| RebornRuntimeError::InvalidArgument {
            reason: format!("could not record trusted laptop access audit event: {error}"),
        })
}

struct ComposedSkillContextSource {
    bundle_source: Arc<FilesystemSkillBundleSource<CompositeRootFilesystem>>,
    source: Arc<dyn HostSkillContextSource>,
    activation_source: Arc<ComposedSelectableSkillContextSource>,
    execution_adapter: Arc<ComposedSkillExecutionAdapter>,
}

const LOCAL_DEV_MAX_SKILL_CONTEXT_TOKENS: usize = 6000;

fn optional_nonzero_u32_env(
    key: &'static str,
) -> Result<Option<std::num::NonZeroU32>, RebornRuntimeError> {
    match std::env::var(key) {
        Ok(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            let parsed =
                trimmed
                    .parse::<u32>()
                    .map_err(|error| RebornRuntimeError::InvalidArgument {
                        reason: format!("{key} must be a positive integer: {error}"),
                    })?;
            if parsed == 0 {
                return Err(RebornRuntimeError::InvalidArgument {
                    reason: format!("{key} must be greater than zero"),
                });
            }
            Ok(std::num::NonZeroU32::new(parsed))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(RebornRuntimeError::InvalidArgument {
            reason: format!("could not read {key}: {error}"),
        }),
    }
}

/// Build the [`SkillActivationSelectorConfig`] used by the local-dev
/// filesystem skill context source. Extracted from
/// [`local_dev_filesystem_skill_context_source`] so the wiring of the
/// `regex_skill_activation_enabled` flag from [`RebornRuntimeInput`] is
/// covered by a unit test (see `tests::local_dev_selector_config_*`).
/// Without this seam the propagation was tested only indirectly through
/// the full [`build_reborn_runtime`] path, where an accidental
/// `..SkillActivationSelectorConfig::default()` regression would slip
/// through silently.
fn local_dev_selector_config(
    regex_skill_activation_enabled: bool,
    injection_mode: SkillInjectionMode,
) -> SkillActivationSelectorConfig {
    SkillActivationSelectorConfig {
        max_context_tokens: LOCAL_DEV_MAX_SKILL_CONTEXT_TOKENS,
        // `ExplicitAndCriteria` (the upstream default) lets a learned skill
        // auto-activate when a later request matches its keywords/patterns —
        // not only when the user types `$name`/`/name`. This is what closes
        // the learn→reuse loop: a skill distilled from one task is applied
        // automatically on the next similar task. Explicit mentions still
        // force-activate; criteria selection is additive and bounded by
        // `max_active_skills` / `max_context_tokens`. Under the default
        // `Listing` injection mode a criteria match ranks the skill in the
        // one-line listing instead of injecting its body.
        selection_mode:
            ironclaw_first_party_extension_ports::SkillActivationSelectionMode::ExplicitAndCriteria,
        regex_activation_enabled: regex_skill_activation_enabled,
        injection_mode,
        ..SkillActivationSelectorConfig::default()
    }
}

/// Parse the Reborn skill-injection mode from the
/// `IRONCLAW_REBORN_SKILL_INJECTION` env switch. Defaults to `listing`
/// (one-line skill listing; bodies load on `builtin.skill_activate`);
/// `full` restores the legacy inject-bodies-by-score behavior.
fn skill_injection_mode_env() -> Result<SkillInjectionMode, RebornRuntimeError> {
    match std::env::var(SKILL_INJECTION_MODE_ENV_KEY) {
        Ok(value) => skill_injection_mode_from(&value),
        Err(std::env::VarError::NotPresent) => Ok(SkillInjectionMode::Listing),
        Err(error) => Err(RebornRuntimeError::InvalidArgument {
            reason: format!("could not read {SKILL_INJECTION_MODE_ENV_KEY}: {error}"),
        }),
    }
}

const SKILL_INJECTION_MODE_ENV_KEY: &str = "IRONCLAW_REBORN_SKILL_INJECTION";

fn skill_injection_mode_from(value: &str) -> Result<SkillInjectionMode, RebornRuntimeError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "listing" => Ok(SkillInjectionMode::Listing),
        "full" => Ok(SkillInjectionMode::Full),
        other => Err(RebornRuntimeError::InvalidArgument {
            reason: format!(
                "{SKILL_INJECTION_MODE_ENV_KEY} must be \"listing\" or \"full\", got {other:?}"
            ),
        }),
    }
}

fn local_dev_filesystem_skill_context_source(
    local_runtime: &crate::factory::RebornRuntimeSubstrate,
    tenant_id: &TenantId,
    regex_skill_activation_enabled: bool,
) -> Result<ComposedSkillContextSource, RebornRuntimeError> {
    let extension = FirstPartySkillsExtension::new(
        Arc::clone(&local_runtime.skill_filesystem),
        FirstPartySkillsExtensionHandles::without_tenant_shared().map_err(|reason| {
            RebornRuntimeError::InvalidArgument {
                reason: format!("first-party skills extension handles: {reason}"),
            }
        })?,
        tenant_id.clone(),
    )
    .map_err(|reason| RebornRuntimeError::InvalidArgument {
        reason: format!("first-party skills extension source: {reason}"),
    })?;
    let selector_config =
        local_dev_selector_config(regex_skill_activation_enabled, skill_injection_mode_env()?);
    let selectable_skills = extension.selectable_skill_runtime_with_setup_markers(
        selector_config,
        Arc::clone(&local_runtime.workspace_filesystem),
        Arc::clone(&local_runtime.skill_auto_activate_learned),
    );
    let bundle_source = extension.bundle_source();
    Ok(ComposedSkillContextSource {
        source: selectable_skills.host_skill_context_source(),
        activation_source: selectable_skills.activation_source(),
        execution_adapter: selectable_skills.execution_adapter(),
        bundle_source,
    })
}

/// Overlay the stored LLM key (if any) onto a clone of `llm`, scoped to
/// feeding [`bootstrap_nearai_mcp_from_effective_llm`]'s `api_key` presence
/// check (it inspects the config directly, not the live provider). NOT the
/// general "stored key -> live provider" mechanism — that's
/// [`RebornLlmReloadAdapter::reload`], invoked once after boot construction.
async fn overlay_stored_llm_key_for_nearai_mcp_bootstrap(
    llm: Option<ResolvedRebornLlm>,
    services: &RebornServices,
) -> Result<Option<ResolvedRebornLlm>, RebornRuntimeError> {
    let Some(mut llm) = llm else {
        return Ok(None);
    };

    let keys = crate::LlmKeyStore::new(services.secret_store());
    if let Some(stored) = keys
        .read(llm.provider_id())
        .await
        .map_err(|error| RebornRuntimeError::LlmProvider(error.to_string()))?
    {
        crate::llm_admin::llm_catalog::apply_stored_api_key(&mut llm.config, stored);
    }

    Ok(Some(llm))
}

async fn bootstrap_nearai_mcp_from_effective_llm(
    services: &RebornServices,
    llm: Option<&ResolvedRebornLlm>,
    owner_scope: ResourceScope,
) -> Result<(), RebornRuntimeError> {
    let Some(llm) = llm else {
        return Ok(());
    };
    let Some(config) =
        crate::llm_admin::nearai_mcp::nearai_mcp_bootstrap_config_from_llm_config(&llm.config)
            .await
            .map_err(|error| RebornRuntimeError::InvalidArgument {
                reason: format!("NEAR AI MCP bootstrap config: {error}"),
            })?
    else {
        return Ok(());
    };
    if let Err(error) = config.endpoint() {
        tracing::debug!(
            %error,
            "NEAR AI MCP auto-bootstrap skipped because the resolved LLM endpoint is not MCP-compatible"
        );
        return Ok(());
    }
    let Some(product_auth) = services.product_auth.as_ref() else {
        return Ok(());
    };
    let Some(extension_management) = services
        .local_runtime
        .as_ref()
        .and_then(|local_runtime| local_runtime.extension_management.as_ref())
    else {
        return Ok(());
    };
    let outcome = crate::llm_admin::nearai_mcp::bootstrap_nearai_mcp(
        Some(config),
        product_auth,
        extension_management,
        owner_scope,
    )
    .await
    .map_err(|error| RebornRuntimeError::InvalidArgument {
        reason: format!("NEAR AI MCP bootstrap from LLM config failed: {error}"),
    })?;
    outcome.log_completion();
    Ok(())
}

struct ValidatedRuntimeIdentity {
    tenant_id: TenantId,
    agent_id: AgentId,
    source_binding_ref: SourceBindingRef,
    reply_target_binding_ref: ReplyTargetBindingRef,
}

fn validate_runtime_identity(
    identity: RebornRuntimeIdentity,
) -> Result<ValidatedRuntimeIdentity, RebornRuntimeError> {
    let tenant_id = TenantId::new(identity.tenant_id).map_err(|reason| {
        RebornRuntimeError::InvalidArgument {
            reason: format!("tenant id: {reason}"),
        }
    })?;
    let agent_id =
        AgentId::new(identity.agent_id).map_err(|reason| RebornRuntimeError::InvalidArgument {
            reason: format!("agent id: {reason}"),
        })?;
    let source_binding_ref =
        SourceBindingRef::new(identity.source_binding_id).map_err(|reason| {
            RebornRuntimeError::InvalidArgument {
                reason: format!("source binding id: {reason}"),
            }
        })?;
    let reply_target_binding_ref = ReplyTargetBindingRef::new(identity.reply_target_binding_id)
        .map_err(|reason| RebornRuntimeError::InvalidArgument {
            reason: format!("reply target binding id: {reason}"),
        })?;
    Ok(ValidatedRuntimeIdentity {
        tenant_id,
        agent_id,
        source_binding_ref,
        reply_target_binding_ref,
    })
}

struct AllowAllCapabilitySurfaceResolver;

#[async_trait::async_trait]
impl CapabilitySurfaceProfileResolver for AllowAllCapabilitySurfaceResolver {
    async fn resolve(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<CapabilityAllowSet, CapabilityResolveError> {
        Ok(CapabilityAllowSet::All)
    }
}

/// Build the production model gateway. Cfg-gated so off-feature builds
/// short-circuit to the stub without referencing types that don't exist.
///
/// Cold boot ALWAYS starts from the placeholder-backed swappable gateway,
/// even when an LLM was resolved at boot — the effective config (including
/// any stored key) is applied exactly once, right after construction,
/// through the same live-reload path the settings UI uses
/// (`RebornLlmReloadAdapter::reload`). No cost table is derived here: there's
/// no real model to cost until that reload swaps in a real provider.
///
/// `provider_factory` is the caller's optional instrumentation decorator
/// (e.g. a benchmark harness layering token/reasoning capture) carried on the
/// resolved LLM. It wraps the *swappable* provider, so the wrapper stays in the
/// call path across the boot-time reload that swaps a real provider into the
/// placeholder (see [`wrap_swappable_gateway`]). Without threading it here the
/// `ResolvedRebornLlm::with_provider_factory` seam would be silently dropped on
/// the cold-boot path.
async fn build_production_model_gateway(
    provider_factory: Option<crate::runtime_input::RebornProviderFactory>,
) -> Result<
    (
        Arc<dyn ironclaw_loop_host::HostManagedModelGateway>,
        Option<ironclaw_loop_host::StaticModelCostTable>,
        Option<RebornLlmReloadParts>,
    ),
    RebornRuntimeError,
> {
    let LlmGatewayBundle {
        gateway, reload, ..
    } = build_placeholder_llm_gateway(provider_factory).await?;
    Ok((gateway, None, Some(reload)))
}

/// Build a dedicated provider for the skill-learning model, when configured.
///
/// Skill distillation/refinement runs against a STRONGER model than the run's.
/// The model id comes from `IRONCLAW_SKILL_LEARNING_MODEL`; it reuses the run's
/// NEAR AI credentials/base URL with only the model overridden (NEAR AI is
/// multi-model and honours a per-request model override). Returns `None` when
/// unconfigured, when the backend is not NEAR AI, or when provider construction
/// fails — in all of which cases skill learning stays disabled.
async fn build_skill_learning_provider(
    config: &ironclaw_llm::LlmConfig,
) -> Option<(Arc<dyn ironclaw_llm::LlmProvider>, String)> {
    let model = std::env::var("IRONCLAW_SKILL_LEARNING_MODEL")
        .ok()
        .filter(|model| !model.trim().is_empty())?;
    if !matches!(config.backend.as_str(), "nearai" | "near_ai" | "near") {
        tracing::debug!(
            backend = %config.backend,
            "skill-learning: learning model is only wired for the nearai backend; skill learning disabled"
        );
        return None;
    }
    let mut nearai = config.nearai.clone();
    nearai.model = model.clone();
    let session = ironclaw_llm::create_session_manager(config.session.clone()).await;
    match ironclaw_llm::create_llm_provider_with_config(
        &nearai,
        session,
        config.request_timeout_secs,
    ) {
        Ok(provider) => Some((provider, model)),
        Err(error) => {
            tracing::debug!(%error, "skill-learning: could not build the learning provider; skill learning disabled");
            None
        }
    }
}

struct LlmGatewayBundle {
    gateway: Arc<dyn ironclaw_loop_host::HostManagedModelGateway>,
    /// Hot-swap handle + session for the live-reload path. The model gateway
    /// wraps a [`SwappableLlmProvider`], so the settings service can rebuild
    /// the provider chain from updated config and atomically swap the inner
    /// backend without rebuilding the gateway or restarting the binary.
    reload: RebornLlmReloadParts,
}

/// The pieces the LLM-config settings service needs to hot-swap the running
/// provider: the reload handle wrapping the live `SwappableLlmProvider`, and
/// the session manager to rebuild the chain against.
pub(crate) struct RebornLlmReloadParts {
    pub(crate) reload_handle: Arc<ironclaw_llm::LlmReloadHandle>,
    pub(crate) session: Arc<ironclaw_llm::SessionManager>,
    pub(crate) nearai_login_states:
        Arc<crate::llm_admin::llm_config_service::NearAiLoginStateStore>,
}

/// Cold-boot gateway: no LLM configured yet. Wraps a placeholder provider (which
/// errors until swapped) so the model-gateway + reload seam exist from the
/// start; the first configuration applied through the settings UI swaps the
/// placeholder for a real provider chain with no restart.
///
/// `provider_factory` is the caller's optional instrumentation decorator. It is
/// applied over the *swappable* wrapper (not the placeholder), so it survives
/// the boot-time reload that swaps in the real provider — the reload-stable
/// contract documented on [`wrap_swappable_gateway`].
async fn build_placeholder_llm_gateway(
    provider_factory: Option<crate::runtime_input::RebornProviderFactory>,
) -> Result<LlmGatewayBundle, RebornRuntimeError> {
    let session =
        ironclaw_llm::create_session_manager(ironclaw_llm::SessionConfig::default()).await;
    let raw: Arc<dyn ironclaw_llm::LlmProvider> = Arc::new(PlaceholderLlmProvider);
    wrap_swappable_gateway(raw, session, provider_factory)
}

/// Wrap a raw provider in a [`SwappableLlmProvider`] + reload handle and build
/// the model gateway. Shared by the real and placeholder boot paths so both get
/// an identical live-reload seam.
///
/// The optional `provider_factory` (caller instrumentation, e.g. token/reasoning
/// capture) is applied **over the swappable wrapper**, so the gateway drives
/// `factory(swappable)`. A live config reload swaps the *inner* of the swappable
/// via the reload handle; because the factory wraps the swappable itself, its
/// instrumentation stays in the call path and continues to observe model calls
/// against the reloaded provider. (Applying the factory to the bare provider
/// instead would let the first reload silently drop the wrapper.)
fn wrap_swappable_gateway(
    raw: Arc<dyn ironclaw_llm::LlmProvider>,
    session: Arc<ironclaw_llm::SessionManager>,
    provider_factory: Option<crate::runtime_input::RebornProviderFactory>,
) -> Result<LlmGatewayBundle, RebornRuntimeError> {
    use ironclaw_llm::{LlmProvider, LlmReloadHandle, SwappableLlmProvider};
    use ironclaw_runner::model_gateway::{LlmModelProfilePolicy, LlmProviderModelGateway};
    use ironclaw_turns::run_profile::ModelProfileId;

    let swappable = Arc::new(SwappableLlmProvider::new(raw));
    let reload_handle = Arc::new(LlmReloadHandle::new(Arc::clone(&swappable), None));
    let swappable_provider: Arc<dyn LlmProvider> = swappable;
    // Gateway drives the factory's wrapper over the swappable (reload-stable);
    // with no factory it drives the swappable directly.
    let provider: Arc<dyn LlmProvider> = match provider_factory {
        Some(factory) => factory(Arc::clone(&swappable_provider)),
        None => swappable_provider,
    };

    let model_profile_id = ModelProfileId::new("interactive_model").map_err(|reason| {
        RebornRuntimeError::LlmProvider(format!("invalid interactive model profile id: {reason}"))
    })?;
    let policy = LlmModelProfilePolicy::new().allow_model_profile(model_profile_id, None);
    let gateway = LlmProviderModelGateway::new(provider, policy);
    Ok(LlmGatewayBundle {
        gateway: Arc::new(gateway),
        reload: RebornLlmReloadParts {
            reload_handle,
            session,
            nearai_login_states: Arc::new(
                crate::llm_admin::llm_config_service::NearAiLoginStateStore::new(),
            ),
        },
    })
}

/// Stand-in provider used before any LLM is configured. Every call fails with a
/// clear, user-safe message; it exists only so the gateway/reload seam is live
/// from a cold boot and the first configuration can swap it out.
#[derive(Debug)]
struct PlaceholderLlmProvider;

#[async_trait::async_trait]
impl ironclaw_llm::LlmProvider for PlaceholderLlmProvider {
    fn model_name(&self) -> &str {
        "unconfigured"
    }

    fn cost_per_token(&self) -> (rust_decimal::Decimal, rust_decimal::Decimal) {
        (rust_decimal::Decimal::ZERO, rust_decimal::Decimal::ZERO)
    }

    async fn complete(
        &self,
        _request: ironclaw_llm::CompletionRequest,
    ) -> Result<ironclaw_llm::CompletionResponse, ironclaw_llm::LlmError> {
        Err(placeholder_unconfigured_error())
    }

    async fn complete_with_tools(
        &self,
        _request: ironclaw_llm::ToolCompletionRequest,
    ) -> Result<ironclaw_llm::ToolCompletionResponse, ironclaw_llm::LlmError> {
        Err(placeholder_unconfigured_error())
    }
}

fn placeholder_unconfigured_error() -> ironclaw_llm::LlmError {
    ironclaw_llm::LlmError::RequestFailed {
        provider: ironclaw_llm::UNCONFIGURED_PROVIDER_ID.to_string(),
        reason: "no LLM provider is configured yet; choose one in Settings → Inference".to_string(),
    }
}

#[cfg(test)]
#[path = "runtime/tests/core.rs"]
mod tests;
