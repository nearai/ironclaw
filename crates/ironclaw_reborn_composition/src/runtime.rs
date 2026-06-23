//! Assembled Reborn runtime: substrate + drivers + worker, started as one.
//!
//! This module is the "later slice" the crate-level docstring promises:
//! product-level wiring on top of the substrate facades exposed by
//! `build_reborn_services`. It is the **only** place in the workspace where
//! `ironclaw_reborn` (drivers, host factory, model gateway bridge),
//! `ironclaw_threads` (session thread service), and (under the
//! `root-llm-provider` feature) `ironclaw_llm` are composed into a running
//! agent.
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
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_filesystem::RootFilesystem;
use ironclaw_first_party_extension_ports::{
    FirstPartySkillsExtension, FirstPartySkillsExtensionHandles, SelectableSkillContextSource,
    SkillActivationSelectorConfig, SkillExecutionAdapter,
};
use ironclaw_host_api::{
    ActionResultSummary, ActionSummary, AgentId, ApprovalRequestId, AuditEnvelope, AuditEventId,
    AuditStage, CapabilityId, CorrelationId, DecisionSummary, EffectKind, InvocationId,
    ResourceScope, TenantId, ThreadId, UserId,
};
use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileResolver,
    EmptyUserProfileSource, FilesystemSkillBundleSource, HostIdentityContextSource,
    HostSkillContextSource, HostUserProfileSource, JsonSpawnSubagentInputCodec,
    LoopCapabilityInputResolver, LoopCapabilityPortFactory, LoopCapabilityResultWriter,
    ModelGatewayBackedSystemInferencePort,
};
use ironclaw_product_adapters::ProjectionStream;
use ironclaw_product_workflow::{
    ApprovalBlockedTurnRun, ApprovalInteractionScope, ApprovalInteractionService,
    ApprovalResolverPort, ApprovalTurnRunLocator, AuthInteractionService,
    DefaultApprovalInteractionService, DefaultAuthInteractionService,
    OutboundPreferencesProductFacade, RunStateApprovalInteractionReadModel,
};
use ironclaw_reborn::loop_exit_applier::{
    ApprovalGateEvidenceStore, ThreadCheckpointLoopExitEvidencePort,
};
use ironclaw_reborn::milestone_events::{
    DurableLoopHostMilestoneScope, DurableLoopHostMilestoneSink,
};
use ironclaw_reborn::runtime::{
    DefaultPlannedRuntimeBuildError, DefaultPlannedRuntimeConfig, DefaultPlannedRuntimeParts,
    RuntimeSubagentGoalStore, RuntimeTurnStateStore, build_default_planned_runtime,
};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_reborn::subagent::goal_store::FilesystemSubagentGoalStore;
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
use ironclaw_reborn::subagent::goal_store::InMemoryBoundedSubagentGoalStore;
use ironclaw_reborn::subagent::{
    flavors::StaticSubagentDefinitionResolver, gate_resolution::BoundedSubagentGateResolutionStore,
};
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, MessageContent, MessageKind, MessageStatus,
    SessionThreadService, ThreadHistoryRequest, ThreadScope,
};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, GetRunStateRequest, IdempotencyKey,
    InMemoryTurnStateStoreLimits, LoopGateRef, ReplyTargetBindingRef, RunProfileResolutionRequest,
    SanitizedCancelReason, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse, TurnActor,
    TurnCoordinator, TurnError, TurnEventProjectionSource, TurnId, TurnPersistenceSnapshot,
    TurnRunId, TurnRunRecord, TurnRunState, TurnRunWake, TurnScope, TurnSpawnTreeStateStore,
    TurnStatus,
    events::EventCursor,
    run_profile::{LoopHostMilestoneSink, LoopRunContext},
};

use ironclaw_host_runtime::MemoryBackedUserProfileSource;
#[cfg(any(test, feature = "test-support"))]
use ironclaw_product_workflow::{
    RebornOutboundDeliveryTargetCapabilities, RebornOutboundDeliveryTargetId,
    RebornOutboundDeliveryTargetSummary, RebornServicesError, WebUiAuthenticatedCaller,
};
use ironclaw_turns::run_profile::UserProfileContext;

use self::runtime_turn_scheduler::RuntimeTurnScheduler;
use crate::default_system_prompt::DefaultSystemPromptIdentitySource;
use crate::factory::{LocalDevRootFilesystem, LocalDevTurnStateStore, builtin_extension_registry};
use crate::local_dev_capability_policy::local_dev_capability_policy;
#[cfg(any(test, feature = "test-support"))]
use crate::outbound_preferences::OutboundDeliveryTargetEntry;
use crate::outbound_preferences::{
    MutableOutboundDeliveryTargetRegistry, OutboundDeliveryTargetProvider,
    OutboundDeliveryTargetRegistrationOutcome, RebornOutboundPreferencesFacade,
};
use crate::projection::{RebornProjectionServices, build_reborn_projection_services};

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone)]
struct StaticOutboundDeliveryTargetProvider {
    entry: OutboundDeliveryTargetEntry,
}

#[cfg(any(test, feature = "test-support"))]
#[async_trait::async_trait]
impl OutboundDeliveryTargetProvider for StaticOutboundDeliveryTargetProvider {
    async fn list_outbound_delivery_targets(
        &self,
        _caller: &WebUiAuthenticatedCaller,
    ) -> Result<Vec<OutboundDeliveryTargetEntry>, RebornServicesError> {
        Ok(vec![self.entry.clone()])
    }
}
use crate::runtime_input::{
    PollSettings, RebornRuntimeIdentity, RebornRuntimeInput, TriggerPollerAuthorizerConfig,
    TriggerPollerSettings,
};
#[cfg(any(test, feature = "test-support"))]
use crate::trigger_poller::TenantScopedTrustedTriggerFireAuthorizer;
use crate::trigger_poller::{
    AccessCheckerTriggerFireAuthorizer, ConversationContentRefMaterializer,
    LocalTriggerTurnSnapshotSource, SnapshotActiveRunLookup, TRIGGER_POLLER_SHUTDOWN_TIMEOUT,
    TriggerPollerCompositionDeps, TriggerPollerRuntimeHandle, TriggerTurnSnapshotSource,
    spawn_trigger_poller,
};
use crate::{
    RebornBuildError, RebornCompositionProfile, RebornProductAuthServices, RebornReadiness,
    RebornReadinessState, RebornServices, build_reborn_services,
};
use production::{
    EmptyCapabilitySurfaceResolver, EmptyIdentityContextSource,
    UnavailableApprovalInteractionService, UnavailableCapabilityIo,
    UnavailableCapabilityPortFactory,
};

const MAX_DESCENDANT_CANCEL_NODES: usize = 1_000;

// Adapter: wraps `MemoryBackedUserProfileSource` (in `ironclaw_host_runtime`) and
// implements `HostUserProfileSource` (in `ironclaw_loop_support`). A direct
// `impl HostUserProfileSource for MemoryBackedUserProfileSource` is forbidden by
// the orphan rule — neither the trait nor the type is defined in this crate. The
// newtype wrapper is defined here, so the impl is allowed. This mirrors how
// `WorkspaceIdentityContextSource` (defined in `src/workspace/`) implements
// `HostIdentityContextSource` (defined in `ironclaw_loop_support`) — the impl
// lives in the crate that owns the *concrete type* and can see the trait.
struct MemoryBackedUserProfileSourceAdapter(MemoryBackedUserProfileSource);

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
    local_runtime: Option<&'a crate::factory::RebornLocalRuntimeServices>,
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
    trigger_repository: Option<Arc<dyn ironclaw_triggers::TriggerRepository>>,
}

fn local_runtime_parts(
    local_runtime: &crate::factory::RebornLocalRuntimeServices,
) -> RuntimeStoreParts<'_> {
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    let subagent_goal_store = Arc::new(FilesystemSubagentGoalStore::new(Arc::clone(
        &local_runtime.subagent_goal_filesystem,
    ))) as Arc<dyn RuntimeSubagentGoalStore>;
    #[cfg(not(any(feature = "libsql", feature = "postgres")))]
    let subagent_goal_store =
        Arc::new(InMemoryBoundedSubagentGoalStore::new()) as Arc<dyn RuntimeSubagentGoalStore>;

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
        trigger_repository: Some(Arc::clone(&local_runtime.trigger_repository)),
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn production_runtime_parts<F>(
    graph: &Arc<crate::factory::RebornProductionRuntimeStoreGraph<F>>,
) -> RuntimeStoreParts<'static>
where
    F: RootFilesystem + 'static,
{
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
        subagent_goal_store: Arc::new(FilesystemSubagentGoalStore::new(Arc::clone(
            &graph.scoped_filesystem,
        ))) as Arc<dyn RuntimeSubagentGoalStore>,
        trigger_repository: Some(Arc::clone(&graph.trigger_repository)),
    }
}

fn enforce_runtime_cutover_gate(
    profile: RebornCompositionProfile,
    readiness: &RebornReadiness,
) -> Result<(), RebornRuntimeError> {
    match profile {
        RebornCompositionProfile::Production => {
            if readiness.state != RebornReadinessState::ProductionValidated {
                return Err(RebornRuntimeError::InvalidArgument {
                    reason: format!(
                        "profile=production cannot start Reborn runtime before production readiness is validated; state={:?}",
                        readiness.state
                    ),
                });
            }
            if let Some(diagnostic) = readiness
                .diagnostics
                .iter()
                .find(|diagnostic| diagnostic.blocks_production)
            {
                return Err(RebornRuntimeError::InvalidArgument {
                    reason: format!(
                        "profile=production cannot start Reborn runtime while readiness diagnostic blocks production: component={:?}, reason={:?}",
                        diagnostic.component, diagnostic.reason
                    ),
                });
            }
            Ok(())
        }
        RebornCompositionProfile::MigrationDryRun => Err(RebornRuntimeError::InvalidArgument {
            reason:
                "profile=migration-dry-run validates production-shaped wiring but must not start live Reborn runtime traffic"
                    .to_string(),
        }),
        RebornCompositionProfile::Disabled => Err(RebornRuntimeError::InvalidArgument {
            reason: "profile=disabled must not start live Reborn runtime traffic".to_string(),
        }),
        RebornCompositionProfile::HostedSingleTenant => {
            if readiness.state != RebornReadinessState::HostedSingleTenantValidated {
                return Err(RebornRuntimeError::InvalidArgument {
                    reason: format!(
                        "profile=hosted-single-tenant cannot start Reborn runtime before hosted single-tenant readiness is validated; required_state=HostedSingleTenantValidated, state={:?}",
                        readiness.state
                    ),
                });
            }
            Ok(())
        }
        RebornCompositionProfile::LocalDev | RebornCompositionProfile::LocalDevYolo => Ok(()),
    }
}

/// Guard: production and migration-dry-run compositions always pre-mint
/// [`SchedulerWakeWiring`] in `build_production_shaped` so the
/// `HostRuntimeServices` notifier and the scheduler wake loop share exactly one
/// channel. If the wiring is `None` for those profiles it means the composition
/// contract was violated (e.g. a code path forgot to mint it), and starting the
/// runtime would silently create a divergent scheduler-local channel. Extracted
/// so the negative branch is unit-testable without a full libsql/postgres
/// substrate.
#[cfg(any(feature = "libsql", feature = "postgres"))]
fn check_production_scheduler_wake_wiring(
    profile: RebornCompositionProfile,
    wiring: &Option<ironclaw_reborn::runtime::SchedulerWakeWiring>,
) -> Result<(), RebornRuntimeError> {
    if wiring.is_none()
        && matches!(
            profile,
            RebornCompositionProfile::Production | RebornCompositionProfile::MigrationDryRun
        )
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
mod local_dev;
#[cfg(test)]
#[path = "runtime/tests/outbound_delivery.rs"]
mod outbound_delivery_tests;
mod production;
mod runtime_turn_scheduler;
mod skills;

#[cfg(test)]
pub(crate) use local_dev::SKILL_ACTIVATE_CAPABILITY_ID;

pub use skills::{
    RebornSkillActivation, RebornSkillActivationMode, RebornSkillAsset, RebornSkillBundle,
    RebornSkillExecutionPlan, RebornSkillExecutionResult, RebornSkillSourceKind,
};

use skills::skill_asset_error;

#[cfg(feature = "root-llm-provider")]
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
/// the WebUI `RebornServicesApi` facade (`resolve_gate`) per the #3094 seam;
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
    #[cfg(feature = "root-llm-provider")]
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
    turn_tree_store: Arc<dyn TurnSpawnTreeStateStore>,
    thread_service: Arc<dyn SessionThreadService>,
    thread_scope: ThreadScope,
    turn_scheduler: RuntimeTurnScheduler,
    trigger_poller_handle: Option<TriggerPollerRuntimeHandle>,
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    credential_refresh_worker_handle:
        Option<crate::credential_refresh_worker::CredentialRefreshWorkerRuntimeHandle>,
    trace_flush_worker: crate::trace_capture::TraceQueueFlushWorkerHandle,
    #[cfg(feature = "root-llm-provider")]
    skill_learning_extraction_tasks:
        Option<Arc<crate::skill_learning::SkillLearningExtractionTasks>>,
    /// Late-binding slot shared with the poller's `PostSubmitHookWrappedSubmitter`.
    /// `set_trigger_post_submit_hook` fills this after `build_reborn_runtime` returns.
    /// `None` when the trigger poller is not enabled.
    #[cfg(feature = "slack-v2-host-beta")]
    post_submit_hook_slot:
        Option<Arc<std::sync::OnceLock<Arc<dyn crate::slack_delivery::PostSubmitDeliveryHook>>>>,
    #[cfg(any(test, feature = "test-support"))]
    trigger_conversation_pairing:
        Option<Arc<dyn ironclaw_conversations::ConversationActorPairingService>>,
    outbound_delivery_target_registry: Option<Arc<MutableOutboundDeliveryTargetRegistry>>,
    budget_event_projection: Option<crate::budget_events::BudgetEventProjection>,
    poll_settings: PollSettings,
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
    skill_activation_source: Option<Arc<LocalDevSelectableSkillContextSource>>,
    skill_execution_adapter: Option<Arc<LocalDevSkillExecutionAdapter>>,
    /// Operator boot config, carried so the WebUI facade can compose the
    /// LLM-config settings service over `providers.json` / `config.toml`.
    #[cfg(feature = "root-llm-provider")]
    boot: Option<ironclaw_reborn_config::RebornBootConfig>,
    /// Hot-swap handle for the live LLM provider, when one was wired at boot.
    #[cfg(feature = "root-llm-provider")]
    llm_reload: Option<RebornLlmReloadParts>,
}

pub(crate) type LocalDevSelectableSkillContextSource =
    SelectableSkillContextSource<FilesystemSkillBundleSource<LocalDevRootFilesystem>>;
type LocalDevSkillExecutionAdapter =
    SkillExecutionAdapter<FilesystemSkillBundleSource<LocalDevRootFilesystem>>;

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
    /// Late-binding slot for the post-submit hook. Created here and shared with
    /// the poller wrapper; filled later by `RebornRuntime::set_trigger_post_submit_hook`
    /// so `build_slack_host_beta_mounts` (called after runtime build) can wire the
    /// hook without restarting the poller.
    #[cfg(feature = "slack-v2-host-beta")]
    post_submit_hook_slot:
        Arc<std::sync::OnceLock<Arc<dyn crate::slack_delivery::PostSubmitDeliveryHook>>>,
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

async fn build_trigger_poller_services(
    local_runtime: &crate::factory::RebornLocalRuntimeServices,
    turn_coordinator: Arc<dyn TurnCoordinator>,
    thread_service: Arc<dyn SessionThreadService>,
    authorizer_config: TriggerPollerAuthorizerConfig,
    access_checker: Option<Arc<dyn crate::runtime_input::TriggerFireAccessChecker>>,
    tenant_id: TenantId,
    default_agent_id: AgentId,
) -> Result<TriggerPollerServices, RebornRuntimeError> {
    let authorizer = build_trigger_fire_authorizer(authorizer_config, access_checker, tenant_id)?;
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    {
        let conversations = local_runtime
            .durable_trigger_conversation_services()
            .await
            .map_err(|error| RebornRuntimeError::InvalidArgument {
                reason: format!("trigger conversation services unavailable: {error}"),
            })?;
        #[cfg(any(test, feature = "test-support"))]
        let pairing_service: Arc<
            dyn ironclaw_conversations::ConversationActorPairingService,
        > = Arc::new(conversations.clone());
        let TriggerPollerServicesInner {
            materializer,
            trusted_submitter,
        } = build_trigger_poller_services_from_conversation_services(
            conversations.clone(),
            conversations,
            turn_coordinator,
            thread_service,
            default_agent_id,
            authorizer,
        );
        Ok(TriggerPollerServices {
            materializer,
            trusted_submitter,
            #[cfg(feature = "slack-v2-host-beta")]
            post_submit_hook_slot: Arc::new(std::sync::OnceLock::new()),
            #[cfg(any(test, feature = "test-support"))]
            pairing_service,
        })
    }
    #[cfg(not(any(feature = "libsql", feature = "postgres")))]
    {
        let conversations = local_runtime.trigger_conversation_services.clone();
        #[cfg(any(test, feature = "test-support"))]
        let pairing_service: Arc<
            dyn ironclaw_conversations::ConversationActorPairingService,
        > = Arc::new(conversations.clone());
        let TriggerPollerServicesInner {
            materializer,
            trusted_submitter,
        } = build_trigger_poller_services_from_conversation_services(
            conversations.clone(),
            conversations,
            turn_coordinator,
            thread_service,
            default_agent_id,
            authorizer,
        );
        Ok(TriggerPollerServices {
            materializer,
            trusted_submitter,
            #[cfg(feature = "slack-v2-host-beta")]
            post_submit_hook_slot: Arc::new(std::sync::OnceLock::new()),
            #[cfg(any(test, feature = "test-support"))]
            pairing_service,
        })
    }
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
) -> Result<Arc<dyn crate::trigger_poller_trusted_submit::TriggerFireAuthorizer>, RebornRuntimeError>
{
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
                    as Arc<dyn crate::trigger_poller_trusted_submit::TriggerFireAuthorizer>
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
    authorizer: Arc<dyn crate::trigger_poller_trusted_submit::TriggerFireAuthorizer>,
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
    turn_state_store: Arc<LocalDevTurnStateStore>,
) -> Arc<dyn ironclaw_triggers::TriggerActiveRunLookup> {
    let snapshot_source: Arc<dyn TriggerTurnSnapshotSource> =
        Arc::new(LocalTriggerTurnSnapshotSource::new(turn_state_store));
    Arc::new(SnapshotActiveRunLookup::new(snapshot_source))
}

struct LocalDevApprovalTurnRunLocator {
    turn_state: Arc<LocalDevTurnStateStore>,
}

impl LocalDevApprovalTurnRunLocator {
    fn new(turn_state: Arc<LocalDevTurnStateStore>) -> Self {
        Self { turn_state }
    }

    async fn snapshot(
        &self,
    ) -> Result<TurnPersistenceSnapshot, ironclaw_product_workflow::ProductWorkflowError> {
        #[cfg(any(feature = "libsql", feature = "postgres"))]
        {
            self.turn_state
                .persistence_snapshot()
                .await
                .map_err(|error| {
                    tracing::debug!(
                        %error,
                        "approval turn-run locator could not read turn persistence snapshot"
                    );
                    approval_turn_locator_unavailable()
                })
        }
        #[cfg(not(any(feature = "libsql", feature = "postgres")))]
        {
            Ok(self.turn_state.persistence_snapshot())
        }
    }
}

struct LocalDevApprovalGateEvidence {
    approval_requests: Arc<dyn ironclaw_run_state::ApprovalRequestStore>,
}

#[async_trait::async_trait]
impl ApprovalGateEvidenceStore for LocalDevApprovalGateEvidence {
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
impl ApprovalTurnRunLocator for LocalDevApprovalTurnRunLocator {
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
                run.scope == turn_scope
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
                run.scope == turn_scope
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
                        .is_none_or(|stored| stored == &turn_scope)
            })
            .filter_map(|checkpoint| {
                snapshot
                    .runs
                    .iter()
                    .find(|run| {
                        run.run_id == checkpoint.run_id
                            && run.scope == turn_scope
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
    snapshot
        .turns
        .iter()
        .any(|turn| turn.turn_id == run.turn_id && turn.scope == run.scope && turn.actor == *actor)
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn approval_turn_locator_unavailable() -> ironclaw_product_workflow::ProductWorkflowError {
    ironclaw_product_workflow::ProductWorkflowError::Transient {
        reason: "approval turn-run locator unavailable".to_string(),
    }
}

/// Fold legacy pre-#4381 WebUI `user_identities` rows into the canonical
/// identity store. The old store wrote those rows into the same libSQL
/// substrate; reading that SQL table is a substrate-level concern handled
/// here in the host layer (not the identity crate), then each row is bound
/// into the filesystem-backed store so an existing SSO user keeps their
/// `UserId` across upgrade. Idempotent (bind re-points to the same user) and
/// a no-op when the legacy table is absent (fresh installs).
#[cfg(feature = "webui-v2-beta")]
async fn fold_legacy_webui_identities<R>(
    db: &libsql::Database,
    tenant_id: &TenantId,
    store: &R,
) -> Result<(), ironclaw_reborn_identity::RebornIdentityError>
where
    R: ironclaw_reborn_identity::RebornIdentityResolver + ?Sized,
{
    use ironclaw_reborn_identity::{
        ExternalSubjectId, ProviderKind, RebornIdentityError, ResolveExternalIdentity, SurfaceKind,
    };

    fn backend(error: libsql::Error) -> RebornIdentityError {
        RebornIdentityError::Backend(error.to_string())
    }
    fn invalid_key(error: ironclaw_reborn_identity::IdentityKeyError) -> RebornIdentityError {
        RebornIdentityError::Backend(error.to_string())
    }

    let conn = db.connect().map_err(backend)?;
    // Scope the existence-check cursor so it is dropped (read lock released)
    // before any write; a lingering open cursor would block the
    // filesystem-backed writes below with `database is locked`.
    let legacy_table_exists = {
        let mut table = conn
            .query(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'user_identities'",
                (),
            )
            .await
            .map_err(backend)?;
        table.next().await.map_err(backend)?.is_some()
    };
    if !legacy_table_exists {
        return Ok(());
    }

    // Drain the read cursor fully BEFORE writing: the store's writes go
    // through a different libSQL connection on the same file, and an open
    // read cursor here would block them with `database is locked`.
    //
    // Carry the verified-email fields too: the legacy WebUI store recorded
    // `email` / `email_verified`, and dropping them on migration would leave
    // the canonical verified-email index unseeded. A migrated Google user
    // would keep their id for the same provider/subject, but a later GitHub
    // login with the same verified email would find no index and mint a
    // second user — a permanent split. `adopt_migrated_identity` preserves
    // both the user id and the verified-email linkage.
    //
    // This intentionally GRANDFATHERS each row's `email_verified` as recorded
    // under the policy in force when the row was written; the one-time fold
    // does NOT re-validate the legacy email against the CURRENT operator
    // allowlist. That is safe because admission is enforced per login, not
    // per index: every live SSO login is gated by `WebuiUserDirectory` against
    // the current allowed-email-domains BEFORE the resolver is consulted, so a
    // grandfathered index for a domain the operator has since removed is never
    // reached (the login is rejected at admission). Re-gating the migration on
    // the current allowlist would need the allowlist plumbed into this
    // substrate-level fold; admission already bounds exploitability, so the
    // migration faithfully preserves prior verified-email links instead.
    let mut legacy = Vec::new();
    let mut rows = conn
        .query(
            "SELECT provider, provider_user_id, user_id, email, email_verified \
             FROM user_identities",
            (),
        )
        .await
        .map_err(backend)?;
    while let Some(row) = rows.next().await.map_err(backend)? {
        let provider: String = row.get(0).map_err(backend)?;
        let subject: String = row.get(1).map_err(backend)?;
        let user: String = row.get(2).map_err(backend)?;
        let email: Option<String> = row.get(3).map_err(backend)?;
        // Legacy column is an INTEGER (0/1); read as i64 so a NULL or odd
        // encoding fails loud rather than silently coercing to unverified.
        let email_verified: i64 = row.get(4).map_err(backend)?;
        legacy.push((provider, subject, user, email, email_verified != 0));
    }
    drop(rows);
    drop(conn);

    for (provider, subject, user, email, email_verified) in legacy {
        let identity = ResolveExternalIdentity {
            tenant_id: tenant_id.clone(),
            surface_kind: SurfaceKind::Oauth,
            provider_kind: ProviderKind::new(provider).map_err(invalid_key)?,
            provider_instance_id: None,
            external_subject_id: ExternalSubjectId::new(subject).map_err(invalid_key)?,
            email,
            email_verified,
            display_name: None,
        };
        let user_id = UserId::new(user)
            .map_err(|error| RebornIdentityError::InvalidUserId(error.to_string()))?;
        store.adopt_migrated_identity(identity, &user_id).await?;
    }
    Ok(())
}

impl RebornRuntime {
    /// Snapshot of the substrate facades produced by `build_reborn_services`.
    /// Exposed for diagnostics / readiness reporting; **not** for traffic.
    pub fn services(&self) -> &RebornServices {
        &self.services
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
    #[cfg(feature = "root-llm-provider")]
    pub(crate) fn webui_boot_config(&self) -> Option<&ironclaw_reborn_config::RebornBootConfig> {
        self.boot.as_ref()
    }

    /// The runtime's NEAR AI session manager, when an LLM seam is wired. The
    /// LLM-config service uses it so a completed NEAR AI login applies to the
    /// live provider on reload.
    #[cfg(feature = "root-llm-provider")]
    pub(crate) fn webui_llm_session(&self) -> Option<Arc<ironclaw_llm::SessionManager>> {
        self.llm_reload
            .as_ref()
            .map(|parts| Arc::clone(&parts.session))
    }

    /// Shared NEAR AI login-state store. The authenticated start endpoint
    /// issues states and the public callback consumes them.
    #[cfg(feature = "root-llm-provider")]
    pub(crate) fn webui_nearai_login_states(
        &self,
    ) -> Option<Arc<crate::llm_config_service::NearAiLoginStateStore>> {
        self.llm_reload
            .as_ref()
            .map(|parts| Arc::clone(&parts.nearai_login_states))
    }

    /// Public NEAR AI login callback mount for the host ingress to merge via
    /// [`crate::webui_serve::WebuiServeConfig::with_public_route_mount`]. Built
    /// from the runtime's private session/reload/boot so those stay internal.
    /// `None` when no LLM seam or boot config was wired.
    #[cfg(all(feature = "root-llm-provider", feature = "webui-v2-beta"))]
    pub fn nearai_login_callback_mount(&self) -> Option<crate::webui_serve::PublicRouteMount> {
        let boot = self.boot.clone()?;
        let session = self.webui_llm_session()?;
        let reload = self.webui_llm_reload_trigger()?;
        let states = self.webui_nearai_login_states()?;
        Some(crate::nearai_login_serve::nearai_login_callback_mount(
            session, reload, boot, states,
        ))
    }

    /// Live LLM-provider reload trigger for the settings service. Returns the
    /// hot-swap adapter when an LLM provider was wired at boot; otherwise
    /// `None`, in which case config edits persist to disk and apply on the
    /// next restart.
    #[cfg(feature = "root-llm-provider")]
    pub(crate) fn webui_llm_reload_trigger(&self) -> Option<Arc<dyn crate::LlmReloadTrigger>> {
        let boot = self.boot.as_ref()?;
        let parts = self.llm_reload.as_ref()?;
        Some(Arc::new(crate::llm_reload::RebornLlmReloadAdapter::new(
            boot.clone(),
            Arc::clone(&parts.reload_handle),
            Arc::clone(&parts.session),
            crate::LlmKeyStore::new(self.services.secret_store()),
        )))
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
    /// Returns `None` when the runtime was built without a local-runtime
    /// substrate, so callers fail closed instead of synthesizing a second
    /// identity store outside the host-owned substrate.
    #[cfg(feature = "webui-v2-beta")]
    pub async fn open_reborn_identity_resolver(
        &self,
        tenant_id: &TenantId,
    ) -> Option<
        Result<
            Arc<dyn ironclaw_reborn_identity::RebornIdentityResolver>,
            ironclaw_reborn_identity::RebornIdentityError,
        >,
    > {
        let local = self.services.local_runtime.as_ref()?;
        // Build the store on the host scoped filesystem (same substrate
        // boundary as every other durable store), scoped by the runtime-owner
        // caller identity. Data is partitioned by tenant in the record path.
        let store = ironclaw_reborn_identity::FilesystemRebornIdentityStore::new(
            Arc::clone(&local.identity_filesystem),
            self.thread_scope.tenant_id.clone(),
            self.actor_user_id.clone(),
            self.thread_scope.agent_id.clone(),
            self.thread_scope.project_id.clone(),
        );
        // One-time legacy fold: the pre-#4381 WebUI store wrote `user_identities`
        // rows into the same libSQL substrate. Reading that SQL table is a
        // substrate-level concern, so it lives here in the host layer (not the
        // identity crate) and binds each row into the filesystem-backed store.
        #[cfg(feature = "libsql")]
        {
            if let Some(identity_substrate_db) = &local.identity_substrate_db
                && let Err(err) =
                    fold_legacy_webui_identities(identity_substrate_db, tenant_id, &store).await
            {
                return Some(Err(err));
            }
        }
        Some(Ok(
            Arc::new(store) as Arc<dyn ironclaw_reborn_identity::RebornIdentityResolver>
        ))
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

    #[cfg(feature = "slack-v2-host-beta")]
    pub(crate) fn auth_challenge_provider(&self) -> Option<Arc<dyn crate::AuthChallengeProvider>> {
        self.services
            .product_auth
            .as_ref()
            .and_then(|product_auth| product_auth.as_auth_challenge_provider())
    }

    #[cfg(feature = "slack-v2-host-beta")]
    pub(crate) fn blocked_auth_flow_canceller(
        &self,
    ) -> Option<Arc<dyn crate::BlockedAuthFlowCanceller>> {
        self.services
            .product_auth
            .as_ref()
            .and_then(|product_auth| product_auth.as_blocked_auth_flow_canceller())
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

    #[cfg_attr(not(feature = "slack-v2-host-beta"), allow(dead_code))]
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
        let summary = RebornOutboundDeliveryTargetSummary::new(
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
                entry: OutboundDeliveryTargetEntry {
                    summary,
                    capabilities: RebornOutboundDeliveryTargetCapabilities {
                        final_replies: true,
                        gate_prompts: false,
                        auth_prompts: false,
                    },
                    reply_target_binding_ref,
                },
            }),
        )
        .map(|_| ())
    }

    #[cfg_attr(not(feature = "slack-v2-host-beta"), allow(dead_code))]
    pub(crate) fn outbound_delivery_target_provider_key_registered(
        &self,
        provider_key: &str,
    ) -> Result<bool, RebornRuntimeError> {
        let Some(registry) = self.outbound_delivery_target_registry.as_ref() else {
            return Err(RebornRuntimeError::InvalidArgument {
                reason: "outbound delivery target registry unavailable for this runtime"
                    .to_string(),
            });
        };
        registry
            .contains_provider_key(provider_key)
            .map_err(|error| RebornRuntimeError::InvalidArgument {
                reason: format!("outbound delivery target provider lookup failed: {error}"),
            })
    }
    /// Wire the triggered-run delivery hook into the already-spawned trigger
    /// poller. Must be called after [`build_reborn_runtime`] returns and after
    /// the hook itself is constructed (e.g. inside
    /// [`crate::slack_host_beta::build_slack_host_beta_mounts`]). The hook is
    /// idempotent: a second call is silently ignored. Returns `false` when the
    /// trigger poller is not enabled (slot is `None`) or the slot is already
    /// occupied, `true` on first successful set.
    #[cfg(feature = "slack-v2-host-beta")]
    pub fn set_trigger_post_submit_hook(
        &self,
        hook: Arc<dyn crate::slack_delivery::PostSubmitDeliveryHook>,
    ) -> bool {
        let Some(slot) = self.post_submit_hook_slot.as_ref() else {
            tracing::debug!("set_trigger_post_submit_hook: trigger poller not enabled, ignoring");
            return false;
        };
        match slot.set(hook) {
            Ok(()) => true,
            Err(_) => {
                tracing::debug!(
                    "set_trigger_post_submit_hook: slot already occupied, ignoring (idempotent)"
                );
                false
            }
        }
    }

    #[cfg(feature = "slack-v2-host-beta")]
    pub(crate) fn trigger_post_submit_hook_is_set(&self) -> bool {
        self.post_submit_hook_slot
            .as_ref()
            .is_some_and(|slot| slot.get().is_some())
    }

    #[cfg(test)]
    fn webui_approval_audit_sink(&self) -> Arc<InMemoryAuditSink> {
        self.approval_audit_sink.clone()
    }

    pub(crate) fn webui_skill_activation_source(
        &self,
    ) -> Option<Arc<LocalDevSelectableSkillContextSource>> {
        self.skill_activation_source.clone()
    }

    /// Read-write project-scoped workspace filesystem for landing inbound
    /// attachment bytes at paths the agent's file tools can later read back.
    /// `None` when no local runtime is composed.
    ///
    /// This deliberately does NOT reuse `rt.workspace_filesystem`: that handle
    /// is intentionally read-only (it backs setup-marker reads — see
    /// `local_dev_setup_marker_workspace_filesystem_is_read_only`), so writing
    /// an attachment through it fails closed with `PermissionDenied`. Build a
    /// read-write view over the same root using the read-write `workspace_mounts`
    /// the agent's `file_write`/`file_read` tools resolve through, so a landed
    /// attachment is addressable at its recorded `storage_key`.
    pub(crate) fn webui_workspace_filesystem(
        &self,
    ) -> Option<Arc<ironclaw_filesystem::ScopedFilesystem<crate::factory::LocalDevRootFilesystem>>>
    {
        self.services.local_runtime.as_ref().map(|rt| {
            Arc::new(ironclaw_filesystem::ScopedFilesystem::with_fixed_view(
                Arc::clone(&rt.extension_filesystem),
                rt.workspace_mounts.clone(),
            ))
        })
    }

    /// Read-only scoped filesystem spanning every mount the standalone WebUI
    /// filesystem viewer can browse (workspace files + persistent memory), over
    /// the same composite root the agent's tools resolve through. `None` when no
    /// local runtime is composed, or when the browse mount view can't be built.
    ///
    /// Distinct from [`Self::webui_workspace_filesystem`]: that handle is the
    /// read-write workspace-only view used to land attachments, whereas this is
    /// a strictly read-only, multi-mount navigation view.
    pub(crate) fn webui_browse_filesystem(
        &self,
    ) -> Option<Arc<ironclaw_filesystem::ScopedFilesystem<crate::factory::LocalDevRootFilesystem>>>
    {
        let rt = self.services.local_runtime.as_ref()?;
        let view = match crate::local_dev_mounts::browse_mount_view() {
            Ok(view) => view,
            Err(error) => {
                // Built from static aliases/targets, so this should never fail;
                // if it does, log loudly rather than silently disabling the
                // filesystem viewer with a bare `None`.
                tracing::error!(
                    target = "ironclaw_reborn_composition::webui",
                    %error,
                    "failed to build webui browse mount view; filesystem viewer unavailable",
                );
                return None;
            }
        };
        Some(Arc::new(
            ironclaw_filesystem::ScopedFilesystem::with_fixed_view(
                Arc::clone(&rt.extension_filesystem),
                view,
            ),
        ))
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
    /// [`crate::budget_events::BudgetEventProjection`] task wired by
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
    /// Without an LLM gateway wired in (i.e. when this crate is built
    /// without the `root-llm-provider` feature or an LLM config is not
    /// provided), the run will fail and the returned reply will surface
    /// that failure via `status = Failed` and `text = None`.
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
        let submitted = self
            .submit_user_turn(
                conversation,
                text,
                &cancellation,
                capture_skill_execution_plan,
            )
            .await?;

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

        if let Some(skill_activation_source) = &self.skill_activation_source
            && let Err(clear_error) = skill_activation_source
                .clear_accepted_message(&submitted.scope, &submitted.accepted_message_ref)
        {
            if reply.is_ok() {
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
        let _send_guard = send_lock.lock_owned().await;
        // Stopped only when every worker has exited; a single crashed worker must not
        // reject submissions while others run.
        if self.turn_scheduler.is_stopped() {
            return Err(RebornRuntimeError::WorkerStopped);
        }
        let scope = self.turn_scope_for(&conversation.0);
        let accepted = self
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
            .map_err(|error| RebornRuntimeError::ThreadService(error.to_string()))?;

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
            adapter
                .record_user_message_for_execution(
                    scope.clone(),
                    accepted_message_ref.clone(),
                    text,
                )
                .map_err(|error| RebornRuntimeError::TurnSubmission(error.to_string()))?;
        } else if let Some(skill_activation_source) = &self.skill_activation_source {
            skill_activation_source
                .record_user_message(scope.clone(), accepted_message_ref.clone(), text)
                .map_err(|error| RebornRuntimeError::TurnSubmission(error.to_string()))?;
        }

        let response = match self
            .turn_coordinator
            .submit_turn(SubmitTurnRequest {
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
            Ok(response) => response,
            Err(error) => {
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
        self.turn_scheduler.notify(TurnRunWake {
            scope: scope.clone(),
            run_id,
            status: submit_status,
            event_cursor: submit_cursor,
        });

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
        #[cfg(any(feature = "libsql", feature = "postgres"))]
        if let Some(credential_refresh_worker) = self.credential_refresh_worker_handle {
            credential_refresh_worker
                .shutdown(
                    crate::credential_refresh_worker::CREDENTIAL_REFRESH_WORKER_SHUTDOWN_TIMEOUT,
                )
                .await;
        }
        self.trace_flush_worker.shutdown().await;
        #[cfg(feature = "root-llm-provider")]
        if let Some(skill_learning_extraction_tasks) = self.skill_learning_extraction_tasks {
            skill_learning_extraction_tasks.shutdown().await;
        }
        self.turn_scheduler.shutdown().await;
        if let Some(projection) = self.budget_event_projection {
            projection.shutdown().await;
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
        adapter: &SkillExecutionAdapter<FilesystemSkillBundleSource<LocalDevRootFilesystem>>,
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
                self.cancel_run(
                    scope,
                    run_id,
                    SanitizedCancelReason::Timeout,
                    "timeout-cancel",
                )
                .await?;
                return Err(RebornRuntimeError::RunTimeout {
                    timeout: self.poll_settings.max_total,
                });
            }
            tokio::select! {
                _ = cancellation.cancelled() => {
                    self.cancel_run(
                        scope,
                        run_id,
                        SanitizedCancelReason::UserRequested,
                        "caller-cancel",
                    )
                    .await?;
                    return Err(RebornRuntimeError::OperationCancelled);
                }
                _ = tokio::time::sleep(self.poll_settings.interval) => {}
            }
        }
    }

    /// Like [`Self::wait_for_terminal`], but also returns when the run parks on
    /// a user-resolvable gate (auth/approval/resource) instead of polling until
    /// those non-terminal states either resolve or hit `RunTimeout`.
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
            // gate". Only the user-resolvable gates short-circuit recording.
            // `BlockedDependentRun` is an internal wait on a child run (the
            // upstream contract names it `AwaitDependentRun`) — it is not
            // resolvable through the gate facade, so it keeps polling like
            // `Queued`/`Running` until the dependent run completes or the poll
            // budget expires.
            let blocked_on_gate = match state.status {
                TurnStatus::BlockedApproval
                | TurnStatus::BlockedAuth
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
    /// gate *resolution* stays on the WebUI `RebornServicesApi` facade
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
        #[cfg(feature = "root-llm-provider")]
        llm,
        #[cfg(feature = "root-llm-provider")]
        boot,
        runner,
        trigger_poller,
        credential_refresh,
        trigger_fire_access_checker,
        poll,
        identity,
        default_project_id,
        regex_skill_activation_enabled,
        skill_context_source: configured_skill_context_source,
        hooks: hooks_config,
        budget_defaults,
        budget_event_observer,
        trajectory_observer,
        #[cfg(any(test, feature = "test-support"))]
        model_gateway_override,
        #[cfg(any(test, feature = "test-support"))]
        model_cost_table_override,
    } = input;

    let mut services_input = services_input.ok_or(RebornRuntimeError::InvalidArgument {
        reason: "RebornRuntimeInput.services is required".to_string(),
    })?;

    let profile = services_input.profile();
    match profile {
        RebornCompositionProfile::LocalDev
        | RebornCompositionProfile::LocalDevYolo
        | RebornCompositionProfile::HostedSingleTenant
        | RebornCompositionProfile::Production => {}
        RebornCompositionProfile::MigrationDryRun => {
            return Err(RebornRuntimeError::InvalidArgument {
                reason:
                    "profile=migration-dry-run validates production-shaped wiring but must not start live Reborn runtime traffic"
                        .to_string(),
            });
        }
        RebornCompositionProfile::Disabled => {
            return Err(RebornRuntimeError::InvalidArgument {
                reason: "profile=disabled must not start live Reborn runtime traffic".to_string(),
            });
        }
    }
    if services_input.runtime_policy().is_none() {
        return Err(RebornRuntimeError::InvalidArgument {
            reason: "RebornRuntimeInput.services must include a resolved runtime policy"
                .to_string(),
        });
    }

    let validated_identity = validate_runtime_identity(identity)?;
    services_input = services_input.with_local_runtime_identity(
        validated_identity.tenant_id.clone(),
        validated_identity.agent_id.clone(),
    );
    #[cfg(feature = "root-llm-provider")]
    let mut has_nearai_mcp_bootstrap_config = services_input.has_nearai_mcp_bootstrap_config();
    #[cfg(feature = "root-llm-provider")]
    if !has_nearai_mcp_bootstrap_config
        && let Some(llm) = llm.as_ref()
        && let Some(config) =
            crate::nearai_mcp::nearai_mcp_bootstrap_config_from_llm_config(&llm.config)
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
    let turn_state_limits = InMemoryTurnStateStoreLimits {
        max_concurrent_runs_per_user: runner.max_concurrent_runs_per_user,
        max_concurrent_trigger_runs: runner.max_concurrent_trigger_runs,
        max_concurrent_conversation_runs: runner.max_concurrent_conversation_runs,
        ..InMemoryTurnStateStoreLimits::default()
    };
    services_input = services_input.with_turn_state_store_limits(turn_state_limits);
    let actor_user_id =
        UserId::new(owner_id.clone()).map_err(|reason| RebornRuntimeError::InvalidArgument {
            reason: format!("user id: {reason}"),
        })?;
    #[cfg(feature = "root-llm-provider")]
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
    #[cfg(feature = "root-llm-provider")]
    let llm =
        apply_startup_stored_llm_key(llm, crate::LlmKeyStore::new(services.secret_store())).await?;
    #[cfg(feature = "root-llm-provider")]
    if !has_nearai_mcp_bootstrap_config {
        bootstrap_nearai_mcp_from_effective_llm(&services, llm.as_ref(), nearai_mcp_owner_scope)
            .await?;
    }
    enforce_runtime_cutover_gate(profile, &services.readiness)?;

    // Extract the pre-minted scheduler wake wiring from the production composition path
    // (minted in `build_production_shaped`) so it can be handed to
    // `DefaultPlannedRuntimeParts.scheduler_wake_wiring` below. The local-dev path
    // leaves this `None` and `build_default_planned_runtime` mints its own wiring.
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    let production_scheduler_wake = {
        let wiring = services.production_scheduler_wake.take();
        // Production and migration-dry-run mint this in `build_production_shaped` so the
        // `HostRuntimeServices` notifier and the scheduler wake loop share one channel.
        // Fail closed if it is missing rather than let `build_default_planned_runtime`
        // mint a divergent scheduler-local channel (silent contract break).
        check_production_scheduler_wake_wiring(profile, &wiring)?;
        wiring
    };
    #[cfg(not(any(feature = "libsql", feature = "postgres")))]
    let production_scheduler_wake: Option<ironclaw_reborn::runtime::SchedulerWakeWiring> = None;

    let runtime_parts = match profile {
        RebornCompositionProfile::LocalDev
        | RebornCompositionProfile::LocalDevYolo
        | RebornCompositionProfile::HostedSingleTenant => {
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
        RebornCompositionProfile::Production => {
            #[cfg(any(feature = "libsql", feature = "postgres"))]
            {
                let production_runtime = services.production_runtime.as_ref().ok_or(
                    RebornRuntimeError::InvalidArgument {
                        reason: "production RebornServices did not provide runtime substrate"
                            .to_string(),
                    },
                )?;
                match production_runtime {
                    #[cfg(feature = "libsql")]
                    crate::factory::RebornProductionRuntimeServices::LibSql(graph) => {
                        production_runtime_parts(graph)
                    }
                    #[cfg(feature = "postgres")]
                    crate::factory::RebornProductionRuntimeServices::Postgres(graph) => {
                        production_runtime_parts(graph)
                    }
                }
            }
            #[cfg(not(any(feature = "libsql", feature = "postgres")))]
            {
                return Err(RebornRuntimeError::InvalidArgument {
                    reason: "production runtime requires a durable storage feature".to_string(),
                });
            }
        }
        _ => unreachable!("unsupported runtime profile checked above"),
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
        trigger_repository: _trigger_repository,
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

    // Resolve the model gateway in three flat steps so the cfg gates
    // don't multiply into a 4-way permutation:
    //
    // 1. Normalize the test-only override into a plain `Option`.
    //    Off-feature builds get a hard `None` so downstream control flow
    //    stays plain.
    // 2. Build the production gateway + cost table from the LLM config
    //    (cfg-gated helper); without `root-llm-provider` the helper
    //    short-circuits to a stub.
    // 3. The test override wins over the production gateway when set;
    //    the LLM-derived cost table is kept regardless so the
    //    accountant can fire against a stub gateway too.
    // 3. A test gateway override short-circuits the production build entirely:
    //    building a real gateway only to discard it wastes startup work (and, on
    //    the cold-boot path, an LLM session manager), which made
    //    timeout-sensitive tests flaky. When no override is set, build normally.
    // Build the (optional) skill-learning provider from the resolved LLM config
    // BEFORE the gateway consumes `llm`. Distillation/refinement runs against a
    // stronger model (IRONCLAW_SKILL_LEARNING_MODEL), reusing the run's NEAR AI
    // credentials with only the model overridden.
    #[cfg(feature = "root-llm-provider")]
    let skill_learning_provider = match llm.as_ref() {
        Some(resolved) => build_skill_learning_provider(&resolved.config).await,
        None => None,
    };
    #[cfg(all(feature = "root-llm-provider", any(test, feature = "test-support")))]
    let (model_gateway, llm_cost_table, llm_reload) = match model_gateway_override {
        Some(override_gateway) => (override_gateway, None, None),
        None => build_production_model_gateway(llm).await?,
    };
    #[cfg(all(
        feature = "root-llm-provider",
        not(any(test, feature = "test-support"))
    ))]
    let (model_gateway, llm_cost_table, llm_reload) = build_production_model_gateway(llm).await?;
    #[cfg(all(
        not(feature = "root-llm-provider"),
        any(test, feature = "test-support")
    ))]
    let (model_gateway, llm_cost_table) = match model_gateway_override {
        Some(override_gateway) => (override_gateway, None),
        None => build_production_model_gateway()?,
    };
    #[cfg(all(
        not(feature = "root-llm-provider"),
        not(any(test, feature = "test-support"))
    ))]
    let (model_gateway, llm_cost_table) = build_production_model_gateway()?;

    // Resolved cost table is either: the LLM-policy-derived table (real
    // LLM wired), a test override (so tests can drive deterministic
    // prices through stub gateways), or None — in which case the
    // accountant doesn't get built (no spend, no cascade). The test
    // override (when set) wins over the LLM-derived table — the test is
    // being explicit about the prices it wants.
    let llm_cost_table_arc: Option<Arc<dyn ironclaw_loop_support::ModelCostTable>> = llm_cost_table
        .map(|table| Arc::new(table) as Arc<dyn ironclaw_loop_support::ModelCostTable>);
    #[cfg(any(test, feature = "test-support"))]
    let resolved_cost_table = model_cost_table_override.or(llm_cost_table_arc);
    #[cfg(not(any(test, feature = "test-support")))]
    let resolved_cost_table = llm_cost_table_arc;

    // Build the model budget accountant from the resolved cost table plus
    // the local-dev governor. `local-dev-yolo` is the explicit local
    // exception: it inherits host trust and must not pause on budget gates.
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
    > = match (profile, resolved_cost_table) {
        (RebornCompositionProfile::LocalDevYolo, _) => None,
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

    let mut loop_exit_evidence = ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
        Arc::clone(&thread_service),
        Arc::clone(&turn_state_store) as Arc<dyn ironclaw_turns::TurnStateStore>,
        Arc::clone(&loop_checkpoint_store) as Arc<dyn ironclaw_turns::LoopCheckpointStore>,
        thread_scope.clone(),
    );
    if let Some(local_runtime) = local_runtime {
        loop_exit_evidence = loop_exit_evidence.with_approval_gate_evidence(Arc::new(
            LocalDevApprovalGateEvidence {
                approval_requests: Arc::clone(&local_runtime.approval_requests)
                    as Arc<dyn ironclaw_run_state::ApprovalRequestStore>,
            },
        ));
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
    let outbound_delivery_target_registry =
        local_runtime.map(|_| Arc::new(MutableOutboundDeliveryTargetRegistry::default()));
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
    #[cfg(feature = "root-llm-provider")]
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
        local_dev_capability_policy,
        display_previews,
    ) = if local_runtime.is_some() {
        let local_dev_capability_policy =
            Arc::new(local_dev_capability_policy().map_err(|error| {
                tracing::error!(%error, "local-dev capability policy is invalid");
                RebornRuntimeError::InvalidArgument {
                    reason: format!("local-dev capability policy is invalid: {error}"),
                }
            })?);
        let local_dev_capabilities = local_dev::capability_wiring(
            &services,
            Arc::clone(&thread_service) as Arc<dyn SessionThreadService>,
            thread_scope.clone(),
            actor_user_id.clone(),
            Arc::clone(&local_dev_capability_policy),
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
            Some(local_dev_capability_policy),
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
        let third_party_input = crate::hooks::ThirdPartyDiscoveryInput {
            filesystem: local_runtime.extension_filesystem.as_ref(),
            tenant_id: &validated_identity.tenant_id,
        };
        let projection_registry = crate::hooks::build_hook_projection_registry(
            builtin_extension_registry()?,
            Some(third_party_input),
            hooks_config,
        )
        .await
        .map_err(|error| RebornRuntimeError::InvalidArgument {
            reason: format!("hook projection registry assembly failed: {error}"),
        })?;
        crate::hooks::build_hook_dispatcher_builder_factory_for_tenant(
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
    let trace_capture_scopes: crate::trace_capture::ObservedTraceScopes =
        Arc::new(std::sync::Mutex::new(std::collections::BTreeSet::from([
            runtime_owner_trace_scope,
        ])));
    let trace_capture_sink: Arc<dyn ironclaw_turns::TurnEventSink> =
        Arc::new(crate::trace_capture::TraceCaptureTurnEventSink::new(
            Arc::clone(&thread_service),
            Arc::clone(&trace_capture_scopes),
        ));
    // Skill learning shares the turn-end seam with trace capture (composed
    // additively, so the trace-capture path is unchanged). It is active only
    // when a learning model is configured (a stronger model than the run's, via
    // IRONCLAW_SKILL_LEARNING_MODEL); otherwise only trace capture runs.
    #[cfg_attr(not(feature = "root-llm-provider"), allow(unused_mut))]
    let mut turn_event_sinks: Vec<Arc<dyn ironclaw_turns::TurnEventSink>> =
        vec![trace_capture_sink];
    #[cfg(feature = "root-llm-provider")]
    let mut skill_learning_extraction_tasks: Option<
        Arc<crate::skill_learning::SkillLearningExtractionTasks>,
    > = None;
    #[cfg(feature = "root-llm-provider")]
    if let (Some((learning_provider, learning_model)), Some(local_runtime)) =
        (skill_learning_provider, local_runtime)
    {
        let inference: Arc<dyn ironclaw_skill_learning::SkillInferencePort> =
            Arc::new(crate::skill_learning::SkillLearningInferenceAdapter::new(
                learning_provider,
                learning_model,
            ));
        // Reuse the runtime's already-built scoped skill-management port so the
        // learned skill lands exactly where the WebUI lists it and the next run
        // loads it. The writer evolves an existing learned skill in place when a
        // recurring task is re-learned, using the same learning model to refine
        // it (accumulated gotchas, bumped version) instead of accreting siblings.
        let skill_refiner: Arc<dyn crate::skill_learning::SkillRefiner> = Arc::new(
            crate::skill_learning::LlmSkillRefiner::new(Arc::clone(&inference)),
        );
        let skill_writer: Arc<dyn crate::skill_learning::SkillWriter> =
            Arc::new(crate::skill_learning::PortSkillWriter::new(
                Arc::clone(&local_runtime.skill_management),
                skill_refiner,
            ));
        // Live "learned a skill" bubble on the run's thread stream (reuses the
        // SkillActivation projection -> existing chat bubble).
        let skill_learned_notifier: Arc<dyn crate::skill_learning::SkillLearnedNotifier> = Arc::new(
            crate::skill_learning::LiveSkillLearnedNotifier::new(skill_learning_publisher),
        );
        let extraction_tasks = Arc::new(crate::skill_learning::SkillLearningExtractionTasks::new());
        skill_learning_extraction_tasks = Some(Arc::clone(&extraction_tasks));
        turn_event_sinks.push(Arc::new(
            crate::skill_learning::SkillLearningTurnEventSink::new(
                Arc::clone(&thread_service),
                inference,
                skill_writer,
                skill_learned_notifier,
                extraction_tasks,
            ),
        ));
    }
    let turn_event_sink: Arc<dyn ironclaw_turns::TurnEventSink> = Arc::new(
        crate::skill_learning::CompositeTurnEventSink::new(turn_event_sinks),
    );

    let communication_context_provider: Option<
        Arc<dyn ironclaw_turns::run_profile::CommunicationContextProvider>,
    > = match (local_runtime, outbound_preferences_facade.clone()) {
        (Some(local_runtime), Some(outbound_preferences_facade)) => {
            let mut lifecycle_facade = crate::lifecycle::RebornLocalLifecycleFacade::new(
                Arc::clone(&local_runtime.skill_management),
            );
            if let Some(extension_management) = &local_runtime.extension_management {
                lifecycle_facade =
                    lifecycle_facade.with_extension_management(Arc::clone(extension_management));
            }
            Some(Arc::new(
                crate::communication_context::RuntimeCommunicationContextProvider::new(
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

    let planned_runtime_parts = DefaultPlannedRuntimeParts {
        turn_state: Arc::clone(&turn_state_store),
        thread_service: Arc::clone(&thread_service),
        thread_scope: thread_scope.clone(),
        // Read landed attachment bytes back through the project workspace
        // filesystem so the model port can build multimodal image parts for
        // vision-capable models. Only available when a local runtime (and thus a
        // workspace filesystem) is composed.
        attachment_read_port: local_runtime.map(|rt| {
            Arc::new(
                crate::attachment_landing::ProjectScopedAttachmentReader::new(Arc::clone(
                    &rt.workspace_filesystem,
                )),
            ) as Arc<dyn ironclaw_loop_support::LoopAttachmentReadPort>
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
        subagent_gate_store: Arc::new(BoundedSubagentGateResolutionStore::new()),
        subagent_definition_resolver: Arc::new(StaticSubagentDefinitionResolver),
        subagent_spawn_input_codec: Arc::new(JsonSpawnSubagentInputCodec::new(
            capability_input_resolver,
        )),
        subagent_spawn_limits: ironclaw_loop_support::SubagentSpawnLimits::default(),
        loop_exit_evidence,
        config: DefaultPlannedRuntimeConfig {
            heartbeat_interval: runner.heartbeat_interval,
            poll_interval: runner.poll_interval,
            worker_count: runner.worker_count,
            ..DefaultPlannedRuntimeConfig::default()
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
        // `extension_filesystem` is the raw `Arc<LocalDevRootFilesystem>` (=
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
        if let (Some(local_runtime), Some(local_dev_capability_policy)) =
            (local_runtime, local_dev_capability_policy)
        {
            let approval_turn_runs = Arc::new(LocalDevApprovalTurnRunLocator::new(Arc::clone(
                &local_runtime.turn_state,
            )));
            let approval_read_model = Arc::new(RunStateApprovalInteractionReadModel::new(
                local_runtime.approval_requests.clone(),
                approval_turn_runs,
            ));
            let approval_resolver = Arc::new(
                ApprovalResolverPort::new(
                    local_runtime.approval_requests.clone(),
                    local_runtime.capability_leases.clone(),
                )
                .with_audit_sink(approval_audit_sink.clone()),
            );
            Arc::new(
                DefaultApprovalInteractionService::new(
                    approval_read_model,
                    Arc::new(approval::LocalDevApprovalLeaseTermsProvider::new(
                        local_dev_capability_policy,
                        Arc::clone(&local_runtime.extension_registry),
                        local_runtime.workspace_mounts.clone(),
                        local_runtime.skill_mounts.clone(),
                        local_runtime.memory_mounts.clone(),
                        local_runtime.system_extensions_lifecycle_mounts.clone(),
                        local_dev::extension_surface::LocalDevExtensionSurfaceSource::new(
                            local_runtime.extension_management.clone(),
                        ),
                    )),
                    approval_resolver,
                    Arc::clone(&planned_turn_coordinator),
                )
                .with_persistent_policy_store(local_runtime.persistent_approval_policies.clone()),
            )
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

    // `trigger_poller_handle`, `post_submit_hook_slot`, and the test-support
    // `trigger_conversation_pairing_value` are produced atomically inside
    // a single `if trigger_poller.enabled` expression. Avoid a
    // `let mut … = None` sentinel pattern flagged by code review
    // (review f-ptr-3): the `let X;` deferred-init form is single-assign
    // per branch and Rust's borrow checker prevents reads before init.
    let trigger_poller_handle: Option<TriggerPollerRuntimeHandle>;
    #[cfg(feature = "slack-v2-host-beta")]
    let runtime_post_submit_hook_slot: Option<
        Arc<std::sync::OnceLock<Arc<dyn crate::slack_delivery::PostSubmitDeliveryHook>>>,
    >;
    #[cfg(any(test, feature = "test-support"))]
    let trigger_conversation_pairing_value: Option<
        Arc<dyn ironclaw_conversations::ConversationActorPairingService>,
    >;
    if trigger_poller.enabled {
        let local_runtime = local_runtime.ok_or(RebornRuntimeError::InvalidArgument {
            reason: "trigger poller is not wired for production runtime launch".to_string(),
        })?;
        validate_trigger_poller_authorization(
            &trigger_poller,
            trigger_fire_access_checker.as_ref(),
        )?;
        let trigger_poller_services = build_trigger_poller_services(
            local_runtime,
            Arc::clone(&planned_turn_coordinator),
            Arc::clone(&thread_service),
            trigger_poller.authorizer,
            trigger_fire_access_checker.clone(),
            thread_scope.tenant_id.clone(),
            validated_identity.agent_id.clone(),
        )
        .await?;
        let active_run_lookup =
            build_trigger_active_run_lookup(Arc::clone(&local_runtime.turn_state));
        #[cfg(any(test, feature = "test-support"))]
        {
            trigger_conversation_pairing_value =
                Some(Arc::clone(&trigger_poller_services.pairing_service));
        }
        #[cfg(feature = "slack-v2-host-beta")]
        let hook_slot = Arc::clone(&trigger_poller_services.post_submit_hook_slot);
        #[cfg(feature = "slack-v2-host-beta")]
        {
            runtime_post_submit_hook_slot = Some(Arc::clone(&hook_slot));
        }
        trigger_poller_handle = spawn_trigger_poller(
            trigger_poller,
            TriggerPollerCompositionDeps {
                repository: Arc::clone(&local_runtime.trigger_repository),
                materializer: trigger_poller_services.materializer,
                trusted_submitter: trigger_poller_services.trusted_submitter,
                active_run_lookup,
                #[cfg(feature = "slack-v2-host-beta")]
                post_submit_hook_slot: hook_slot,
            },
        )
        .map_err(|error| RebornRuntimeError::InvalidArgument {
            reason: format!("trigger poller could not be started: {error}"),
        })?;
    } else {
        trigger_poller_handle = None;
        #[cfg(feature = "slack-v2-host-beta")]
        {
            runtime_post_submit_hook_slot = None;
        }
        #[cfg(any(test, feature = "test-support"))]
        {
            trigger_conversation_pairing_value = None;
        }
    }
    let scheduler_notifier = composition.scheduler_handle.wake_notifier();

    // Spawn the background Google OAuth credential keepalive worker (B4).
    // Gated on the db features: the worker deps (candidate source + leader lock
    // + refresh port) are only produced together on production paths (libsql /
    // postgres), bundled into `CredentialRefreshWorkerReady::Ready`. Local-dev /
    // override paths are `Absent` and the worker is skipped. The `enabled` policy
    // flag still gates the actual spawn inside `spawn_credential_refresh_worker`.
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    let credential_refresh_worker_handle = match std::mem::replace(
        &mut services.credential_refresh_worker,
        crate::factory::CredentialRefreshWorkerReady::Absent,
    ) {
        crate::factory::CredentialRefreshWorkerReady::Ready {
            candidate_source,
            leader_lock,
            refresh_port,
        } => crate::credential_refresh_worker::spawn_credential_refresh_worker(
            credential_refresh,
            crate::credential_refresh_worker::CredentialRefreshWorkerDeps {
                candidate_source,
                refresh_port,
                leader_lock: std::sync::Arc::new(leader_lock),
            },
        ),
        crate::factory::CredentialRefreshWorkerReady::Absent => None,
    };
    // When no db feature is active, silence the unused-variable warning.
    #[cfg(not(any(feature = "libsql", feature = "postgres")))]
    let _ = credential_refresh;

    let trace_flush_worker =
        crate::trace_capture::spawn_trace_queue_flush_worker(trace_capture_scopes);
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
        crate::budget_events::BudgetEventProjection::spawn(
            broadcast_budget_event_sink.as_ref(),
            observer,
        )
    });

    Ok(RebornRuntime {
        services,
        turn_coordinator,
        turn_tree_store: turn_state_store,
        thread_service,
        thread_scope,
        turn_scheduler: RuntimeTurnScheduler::new(composition.scheduler_handle, scheduler_notifier),
        trigger_poller_handle,
        #[cfg(any(feature = "libsql", feature = "postgres"))]
        credential_refresh_worker_handle,
        trace_flush_worker,
        #[cfg(feature = "root-llm-provider")]
        skill_learning_extraction_tasks,
        #[cfg(feature = "slack-v2-host-beta")]
        post_submit_hook_slot: runtime_post_submit_hook_slot,
        #[cfg(any(test, feature = "test-support"))]
        trigger_conversation_pairing: trigger_conversation_pairing_value,
        outbound_delivery_target_registry,
        budget_event_projection,
        poll_settings: poll,
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
        #[cfg(feature = "root-llm-provider")]
        boot,
        #[cfg(feature = "root-llm-provider")]
        llm_reload,
    })
}

fn build_webui_auth_interaction_service(
    product_auth: Option<&RebornProductAuthServices>,
    turn_state_store: Arc<LocalDevTurnStateStore>,
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
        Arc::new(auth_interaction::LocalDevAuthInteractionReadModel::new(
            turn_state_store,
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

struct LocalDevSkillContextSource {
    source: Arc<dyn HostSkillContextSource>,
    activation_source: Arc<LocalDevSelectableSkillContextSource>,
    execution_adapter: Arc<LocalDevSkillExecutionAdapter>,
}

const LOCAL_DEV_MAX_SKILL_CONTEXT_TOKENS: usize = 6000;

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
) -> SkillActivationSelectorConfig {
    SkillActivationSelectorConfig {
        max_context_tokens: LOCAL_DEV_MAX_SKILL_CONTEXT_TOKENS,
        // `ExplicitAndCriteria` (the upstream default) lets a learned skill
        // auto-activate when a later request matches its keywords/patterns —
        // not only when the user types `$name`/`/name`. This is what closes
        // the learn→reuse loop: a skill distilled from one task is applied
        // automatically on the next similar task. Explicit mentions still
        // force-activate; criteria selection is additive and bounded by
        // `max_active_skills` / `max_context_tokens`.
        selection_mode:
            ironclaw_first_party_extension_ports::SkillActivationSelectionMode::ExplicitAndCriteria,
        regex_activation_enabled: regex_skill_activation_enabled,
        ..SkillActivationSelectorConfig::default()
    }
}

fn local_dev_filesystem_skill_context_source(
    local_runtime: &crate::factory::RebornLocalRuntimeServices,
    tenant_id: &TenantId,
    regex_skill_activation_enabled: bool,
) -> Result<LocalDevSkillContextSource, RebornRuntimeError> {
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
    let selector_config = local_dev_selector_config(regex_skill_activation_enabled);
    let selectable_skills = extension.selectable_skill_runtime_with_setup_markers(
        selector_config,
        Arc::clone(&local_runtime.workspace_filesystem),
        Arc::clone(&local_runtime.skill_auto_activate_learned),
    );
    Ok(LocalDevSkillContextSource {
        source: selectable_skills.host_skill_context_source(),
        activation_source: selectable_skills.activation_source(),
        execution_adapter: selectable_skills.execution_adapter(),
    })
}

#[cfg(feature = "root-llm-provider")]
async fn apply_startup_stored_llm_key(
    llm: Option<ResolvedRebornLlm>,
    keys: crate::LlmKeyStore,
) -> Result<Option<ResolvedRebornLlm>, RebornRuntimeError> {
    let Some(mut llm) = llm else {
        return Ok(None);
    };

    if let Some(stored) = keys
        .read(llm.provider_id())
        .await
        .map_err(|error| RebornRuntimeError::LlmProvider(error.to_string()))?
    {
        crate::llm_catalog::apply_stored_api_key(&mut llm.config, stored);
    }

    Ok(Some(llm))
}

#[cfg(feature = "root-llm-provider")]
async fn bootstrap_nearai_mcp_from_effective_llm(
    services: &RebornServices,
    llm: Option<&ResolvedRebornLlm>,
    owner_scope: ResourceScope,
) -> Result<(), RebornRuntimeError> {
    let Some(llm) = llm else {
        return Ok(());
    };
    let Some(config) = crate::nearai_mcp::nearai_mcp_bootstrap_config_from_llm_config(&llm.config)
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
    let outcome = crate::nearai_mcp::bootstrap_nearai_mcp(
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

/// Build the production model gateway and its (optional) LLM-derived
/// cost table. Cfg-gated so off-feature builds short-circuit to the
/// stub without referencing types that don't exist.
#[cfg(feature = "root-llm-provider")]
async fn build_production_model_gateway(
    llm: Option<crate::runtime_input::ResolvedRebornLlm>,
) -> Result<
    (
        Arc<dyn ironclaw_loop_support::HostManagedModelGateway>,
        Option<ironclaw_loop_support::StaticModelCostTable>,
        Option<RebornLlmReloadParts>,
    ),
    RebornRuntimeError,
> {
    // Even with no LLM configured at boot we build a real swappable gateway
    // around a placeholder provider (which errors until swapped) plus a reload
    // handle. That way the FIRST configuration made through the settings UI
    // hot-swaps the placeholder into a working provider without a restart —
    // otherwise a cold boot would wire a dead stub with no reload seam.
    match llm {
        Some(cfg) => {
            let LlmGatewayBundle {
                gateway,
                policy,
                reload,
            } = build_llm_gateway(cfg).await?;
            Ok((gateway, Some(policy.build_cost_table()), Some(reload)))
        }
        None => {
            let LlmGatewayBundle {
                gateway, reload, ..
            } = build_placeholder_llm_gateway().await?;
            // No cost table for the placeholder: there is no real model to cost,
            // and a synthetic table would gate budgets against a model that
            // isn't actually in use. The budget cost table is (re)derived when a
            // real provider is configured + the binary restarts.
            Ok((gateway, None, Some(reload)))
        }
    }
}

/// Build a dedicated provider for the skill-learning model, when configured.
///
/// Skill distillation/refinement runs against a STRONGER model than the run's.
/// The model id comes from `IRONCLAW_SKILL_LEARNING_MODEL`; it reuses the run's
/// NEAR AI credentials/base URL with only the model overridden (NEAR AI is
/// multi-model and honours a per-request model override). Returns `None` when
/// unconfigured, when the backend is not NEAR AI, or when provider construction
/// fails — in all of which cases skill learning stays disabled.
#[cfg(feature = "root-llm-provider")]
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

#[cfg(not(feature = "root-llm-provider"))]
fn build_production_model_gateway() -> Result<
    (
        Arc<dyn ironclaw_loop_support::HostManagedModelGateway>,
        Option<ironclaw_loop_support::StaticModelCostTable>,
    ),
    RebornRuntimeError,
> {
    Ok((build_stub_gateway(), None))
}

#[cfg(feature = "root-llm-provider")]
struct LlmGatewayBundle {
    gateway: Arc<dyn ironclaw_loop_support::HostManagedModelGateway>,
    /// Policy used to derive the budget accountant's cost table — kept
    /// alongside the gateway so the composer doesn't re-derive the
    /// `ModelProfileId → provider-model` mapping in two places.
    policy: ironclaw_reborn::model_gateway::LlmModelProfilePolicy,
    /// Hot-swap handle + session for the live-reload path. The model gateway
    /// wraps a [`SwappableLlmProvider`], so the settings service can rebuild
    /// the provider chain from updated config and atomically swap the inner
    /// backend without rebuilding the gateway or restarting the binary.
    reload: RebornLlmReloadParts,
}

/// The pieces the LLM-config settings service needs to hot-swap the running
/// provider: the reload handle wrapping the live `SwappableLlmProvider`, and
/// the session manager to rebuild the chain against.
#[cfg(feature = "root-llm-provider")]
pub(crate) struct RebornLlmReloadParts {
    pub(crate) reload_handle: Arc<ironclaw_llm::LlmReloadHandle>,
    pub(crate) session: Arc<ironclaw_llm::SessionManager>,
    pub(crate) nearai_login_states: Arc<crate::llm_config_service::NearAiLoginStateStore>,
}

#[cfg(feature = "root-llm-provider")]
async fn build_llm_gateway(llm: ResolvedRebornLlm) -> Result<LlmGatewayBundle, RebornRuntimeError> {
    let session = ironclaw_llm::create_session_manager(llm.config.session.clone()).await;
    // Config is always the construction source. A caller-supplied factory (e.g.
    // an instrumentation wrapper) then decorates the built provider; without one
    // the config-built provider is driven as-is.
    let built = ironclaw_llm::build_static_provider_chain(&llm.config, Arc::clone(&session))
        .await
        .map_err(|error| RebornRuntimeError::LlmProvider(error.to_string()))?;
    // The factory is applied *inside* `wrap_swappable_gateway` — over the
    // swappable wrapper, not the bare config provider — so a live config reload
    // (which swaps the swappable's inner) keeps the factory's wrapper in the
    // call path. See `wrap_swappable_gateway`.
    wrap_swappable_gateway(built, session, llm.provider_factory.clone())
}

/// Cold-boot gateway: no LLM configured yet. Wraps a placeholder provider (which
/// errors until swapped) so the model-gateway + reload seam exist from the
/// start; the first configuration applied through the settings UI swaps the
/// placeholder for a real provider chain with no restart.
#[cfg(feature = "root-llm-provider")]
async fn build_placeholder_llm_gateway() -> Result<LlmGatewayBundle, RebornRuntimeError> {
    let session =
        ironclaw_llm::create_session_manager(ironclaw_llm::SessionConfig::default()).await;
    let raw: Arc<dyn ironclaw_llm::LlmProvider> = Arc::new(PlaceholderLlmProvider);
    wrap_swappable_gateway(raw, session, None)
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
#[cfg(feature = "root-llm-provider")]
fn wrap_swappable_gateway(
    raw: Arc<dyn ironclaw_llm::LlmProvider>,
    session: Arc<ironclaw_llm::SessionManager>,
    provider_factory: Option<crate::runtime_input::RebornProviderFactory>,
) -> Result<LlmGatewayBundle, RebornRuntimeError> {
    use ironclaw_llm::{LlmProvider, LlmReloadHandle, SwappableLlmProvider};
    use ironclaw_reborn::model_gateway::{LlmModelProfilePolicy, LlmProviderModelGateway};
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
    let gateway = LlmProviderModelGateway::new(provider, policy.clone());
    Ok(LlmGatewayBundle {
        gateway: Arc::new(gateway),
        policy,
        reload: RebornLlmReloadParts {
            reload_handle,
            session,
            nearai_login_states: Arc::new(crate::llm_config_service::NearAiLoginStateStore::new()),
        },
    })
}

/// Stand-in provider used before any LLM is configured. Every call fails with a
/// clear, user-safe message; it exists only so the gateway/reload seam is live
/// from a cold boot and the first configuration can swap it out.
#[cfg(feature = "root-llm-provider")]
#[derive(Debug)]
struct PlaceholderLlmProvider;

#[cfg(feature = "root-llm-provider")]
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

#[cfg(feature = "root-llm-provider")]
fn placeholder_unconfigured_error() -> ironclaw_llm::LlmError {
    ironclaw_llm::LlmError::RequestFailed {
        provider: "unconfigured".to_string(),
        reason: "no LLM provider is configured yet; choose one in Settings → Inference".to_string(),
    }
}

// Only the substrate-only build (no `root-llm-provider`) still wires a dead
// stub gateway. With the LLM provider compiled in, a cold boot uses a
// placeholder-backed swappable gateway instead (see `build_placeholder_llm_gateway`).
#[cfg(not(feature = "root-llm-provider"))]
fn build_stub_gateway() -> Arc<dyn ironclaw_loop_support::HostManagedModelGateway> {
    use async_trait::async_trait;
    use ironclaw_loop_support::{
        HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
        HostManagedModelRequest, HostManagedModelResponse,
    };

    #[derive(Debug, Default)]
    struct StubGateway;

    #[async_trait]
    impl HostManagedModelGateway for StubGateway {
        async fn stream_model(
            &self,
            _request: HostManagedModelRequest,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            Err(HostManagedModelError::safe(
                HostManagedModelErrorKind::Unavailable,
                "no LLM gateway wired (build with `root-llm-provider` feature)",
            ))
        }
    }

    Arc::new(StubGateway)
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;

    use async_trait::async_trait;
    use chrono::Utc;
    use ironclaw_auth::{GOOGLE_CALENDAR_EVENTS_SCOPE, GOOGLE_CALENDAR_READONLY_SCOPE};

    /// Wiring guard: the `regex_skill_activation_enabled` flag from
    /// [`RebornRuntimeInput`] must reach
    /// [`SkillActivationSelectorConfig::regex_activation_enabled`]
    /// unchanged, not get clobbered by a stray
    /// `..SkillActivationSelectorConfig::default()` spread or by the
    /// helper accidentally taking `Default::default()`. Covers the
    /// composition-level path that
    /// [`local_dev_filesystem_skill_context_source`] depends on.
    #[test]
    fn local_dev_selector_config_propagates_regex_activation_disabled() {
        let cfg = super::local_dev_selector_config(false);
        assert!(
            !cfg.regex_activation_enabled,
            "regex_skill_activation_enabled=false must propagate into SkillActivationSelectorConfig"
        );
        // Local-dev uses criteria selection so a learned skill auto-activates on
        // a keyword/pattern match (the learn→reuse loop), not only on an
        // explicit `$name` mention. A revert to `ExplicitOnly` would silently
        // break auto-reuse, so lock it here.
        assert!(matches!(
            cfg.selection_mode,
            ironclaw_first_party_extension_ports::SkillActivationSelectionMode::ExplicitAndCriteria
        ));
    }

    #[test]
    fn local_dev_selector_config_propagates_regex_activation_enabled() {
        let cfg = super::local_dev_selector_config(true);
        assert!(
            cfg.regex_activation_enabled,
            "regex_skill_activation_enabled=true must propagate into SkillActivationSelectorConfig"
        );
    }

    #[test]
    fn local_dev_selector_config_uses_large_skill_context_budget() {
        let cfg = super::local_dev_selector_config(true);
        assert_eq!(
            cfg.max_context_tokens, 6000,
            "local-dev Reborn skill activation should match the legacy 6000-token skill budget"
        );
    }

    fn readiness_for_runtime_gate(
        profile: RebornCompositionProfile,
        state: RebornReadinessState,
        diagnostics: Vec<crate::RebornReadinessDiagnostic>,
    ) -> RebornReadiness {
        RebornReadiness {
            profile,
            state,
            facades: crate::RebornFacadeReadiness {
                host_runtime: true,
                turn_coordinator: true,
                product_auth: true,
            },
            workers: crate::RebornWorkerReadiness {
                turn_runner: true,
                trigger_poller: false,
            },
            diagnostics,
        }
    }

    #[test]
    fn runtime_cutover_gate_allows_validated_production_readiness() {
        let readiness = readiness_for_runtime_gate(
            RebornCompositionProfile::Production,
            RebornReadinessState::ProductionValidated,
            Vec::new(),
        );

        super::enforce_runtime_cutover_gate(RebornCompositionProfile::Production, &readiness)
            .expect("validated production runtime can start");
    }

    #[test]
    fn runtime_cutover_gate_rejects_blocking_production_diagnostic() {
        let readiness = readiness_for_runtime_gate(
            RebornCompositionProfile::Production,
            RebornReadinessState::ProductionValidated,
            vec![
                crate::RebornReadinessDiagnostic::production_blocker(
                    RebornCompositionProfile::Production,
                    crate::RebornReadinessDiagnosticComponent::RuntimePolicy,
                    crate::RebornReadinessDiagnosticReason::LocalOnly,
                )
                .expect("production profile should create a blocker"),
            ],
        );

        let error =
            super::enforce_runtime_cutover_gate(RebornCompositionProfile::Production, &readiness)
                .expect_err("blocking production diagnostic prevents runtime start");
        let RebornRuntimeError::InvalidArgument { reason } = error else {
            panic!("expected invalid argument, got {error:?}");
        };
        assert!(reason.contains("RuntimePolicy"), "reason: {reason}");
        assert!(reason.contains("LocalOnly"), "reason: {reason}");
    }

    #[test]
    fn runtime_cutover_gate_rejects_migration_dry_run_runtime_start() {
        let readiness = readiness_for_runtime_gate(
            RebornCompositionProfile::MigrationDryRun,
            RebornReadinessState::MigrationDryRunValidated,
            Vec::new(),
        );

        let error = super::enforce_runtime_cutover_gate(
            RebornCompositionProfile::MigrationDryRun,
            &readiness,
        )
        .expect_err("migration-dry-run cannot start live runtime");
        let RebornRuntimeError::InvalidArgument { reason } = error else {
            panic!("expected invalid argument, got {error:?}");
        };
        assert!(reason.contains("migration-dry-run"), "reason: {reason}");
    }

    #[test]
    fn runtime_cutover_gate_allows_local_dev_readiness() {
        let readiness = readiness_for_runtime_gate(
            RebornCompositionProfile::LocalDev,
            RebornReadinessState::DevOnly,
            vec![crate::RebornReadinessDiagnostic::local_dev()],
        );

        super::enforce_runtime_cutover_gate(RebornCompositionProfile::LocalDev, &readiness)
            .expect("local-dev runtime is not production traffic");
    }

    #[test]
    fn runtime_cutover_gate_allows_hosted_single_tenant_readiness() {
        let readiness = readiness_for_runtime_gate(
            RebornCompositionProfile::HostedSingleTenant,
            RebornReadinessState::HostedSingleTenantValidated,
            Vec::new(),
        );

        super::enforce_runtime_cutover_gate(
            RebornCompositionProfile::HostedSingleTenant,
            &readiness,
        )
        .expect("validated hosted single-tenant runtime can start");
    }

    #[test]
    fn runtime_cutover_gate_rejects_local_dev_readiness_for_hosted_single_tenant() {
        let readiness = readiness_for_runtime_gate(
            RebornCompositionProfile::HostedSingleTenant,
            RebornReadinessState::DevOnly,
            vec![crate::RebornReadinessDiagnostic::local_dev()],
        );

        let error = super::enforce_runtime_cutover_gate(
            RebornCompositionProfile::HostedSingleTenant,
            &readiness,
        )
        .expect_err("hosted single-tenant runtime requires hosted readiness");
        let RebornRuntimeError::InvalidArgument { reason } = error else {
            panic!("expected invalid argument, got {error:?}");
        };
        assert!(reason.contains("hosted-single-tenant"), "reason: {reason}");
        assert!(
            reason.contains("HostedSingleTenantValidated"),
            "reason: {reason}"
        );
    }

    // ── scheduler wake wiring guard unit tests ───────────────────────────────
    // These exercise `check_production_scheduler_wake_wiring` directly so the
    // fail-closed negative branch is covered without needing a full libsql /
    // postgres substrate.  The guard is gated on the same `libsql | postgres`
    // cfg as the production composition path it protects.

    #[cfg(feature = "libsql")]
    #[test]
    fn production_scheduler_wake_guard_rejects_production_with_absent_wiring() {
        let err = super::check_production_scheduler_wake_wiring(
            RebornCompositionProfile::Production,
            &None,
        )
        .expect_err(
            "production runtime with absent scheduler wake wiring must be rejected fail-closed",
        );
        let RebornRuntimeError::InvalidArgument { reason } = err else {
            panic!("expected InvalidArgument, got {err:?}");
        };
        assert!(
            reason.contains("production runtime missing scheduler wake wiring"),
            "reason should name the missing wiring, got: {reason}"
        );
    }

    #[cfg(feature = "libsql")]
    #[test]
    fn production_scheduler_wake_guard_rejects_migration_dry_run_with_absent_wiring() {
        let err = super::check_production_scheduler_wake_wiring(
            RebornCompositionProfile::MigrationDryRun,
            &None,
        )
        .expect_err(
            "migration-dry-run with absent scheduler wake wiring must be rejected fail-closed",
        );
        let RebornRuntimeError::InvalidArgument { reason } = err else {
            panic!("expected InvalidArgument, got {err:?}");
        };
        assert!(
            reason.contains("production runtime missing scheduler wake wiring"),
            "reason should name the missing wiring, got: {reason}"
        );
    }

    #[cfg(feature = "libsql")]
    #[test]
    fn production_scheduler_wake_guard_passes_local_dev_with_absent_wiring() {
        // Local-dev never mints scheduler wake wiring; the guard must not
        // reject it (the scheduler loop mints its own channel on that path).
        super::check_production_scheduler_wake_wiring(RebornCompositionProfile::LocalDev, &None)
            .expect("local-dev is exempt from the scheduler wake wiring requirement");
    }

    use ironclaw_authorization::CapabilityLeaseStore;
    use ironclaw_events::{EventStreamKey, ReadScope};
    #[cfg(all(feature = "root-llm-provider", feature = "libsql"))]
    use ironclaw_host_api::ProjectId;
    use ironclaw_host_api::{
        Action, AgentId, ApprovalRequest, ApprovalRequestId, AuditStage, CapabilityId,
        CorrelationId, EffectKind, InvocationFingerprint, InvocationId, Principal,
        ResourceEstimate, ResourceScope, TenantId, ThreadId, UserId,
        runtime_policy::{
            ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy,
            FilesystemBackendKind, NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
        },
    };
    use ironclaw_loop_support::{
        HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
        HostManagedModelMessageRole, HostManagedModelRequest, HostManagedModelResponse,
        HostSkillContextBuildError, HostSkillContextCandidate, HostSkillContextSource, ModelCost,
        SpawnSubagentMode, SubagentKindId, SubagentThreadKind, SubagentThreadMetadata,
    };
    use ironclaw_product_adapters::{ProductOutboundPayload, ProductProjectionItem};
    use ironclaw_product_workflow::{
        LifecyclePackageKind, LifecyclePackageRef, LifecyclePhase, LifecycleProductPayload,
        LifecycleReadinessBlocker, RebornExtensionCredentialSetup, RebornServicesErrorCode,
        RebornServicesErrorKind, RebornSetOutboundPreferencesRequest, RebornStreamEventsRequest,
        RebornSubmitTurnResponse, WebUiAuthenticatedCaller, WebUiCreateThreadRequest,
        WebUiListAutomationsRequest, WebUiResolveGateRequest, WebUiSendMessageRequest,
        WebUiSetupExtensionRequest, approval_gate_ref,
    };
    use ironclaw_run_state::ApprovalRequestStore;
    use ironclaw_skills::SkillTrust;
    use ironclaw_threads::{
        AppendToolResultReferenceRequest, EnsureThreadRequest, LoadContextMessagesRequest,
        MessageKind, MessageStatus, ThreadHistoryRequest, ThreadScope, ToolResultSafeSummary,
    };
    use ironclaw_turns::{
        AcceptedMessageRef, AllowAllTurnAdmissionPolicy, BlockedReason, GetRunStateRequest,
        IdempotencyKey, LoopResultRef, ReplyTargetBindingRef, SanitizedCancelReason,
        SourceBindingRef, SubmitChildRunRequest, SubmitTurnRequest, SubmitTurnResponse, TurnActor,
        TurnCheckpointId, TurnId, TurnLeaseToken, TurnRunId, TurnRunnerId, TurnScope, TurnStatus,
        run_profile::{
            InMemoryRunProfileResolver, LoopCapabilityPort, LoopCheckpointStateRef, LoopRunContext,
            ModelProfileId, ProviderToolCall, RunProfileResolutionRequest, RunProfileResolver,
            SkillVisibility, VisibleCapabilityRequest,
        },
        runner::{BlockRunRequest, ClaimRunRequest, TurnRunTransitionPort},
    };
    use rust_decimal_macros::dec;

    #[cfg(feature = "libsql")]
    use crate::RebornRuntimeProcessBinding;
    #[cfg(feature = "libsql")]
    use crate::hooks::HooksActivationConfig;
    use crate::input::RebornBuildInput;
    use crate::runtime_input::{
        PollSettings, RebornRuntimeIdentity, RebornRuntimeInput, TriggerFireAccessCheck,
        TriggerFireAccessChecker, TriggerFireAccessDecision, TriggerFireAccessError,
        TriggerPollerSettings,
    };
    use crate::webui::build_webui_services;
    use crate::{
        RebornCompositionProfile, RebornReadiness, RebornReadinessState, RebornRuntimeError,
        extension_lifecycle::ExtensionActivationMode,
    };

    use super::{
        RebornSkillSourceKind, TRUSTED_LAPTOP_ACCESS_AUDIT_KIND,
        TRUSTED_LAPTOP_ACCESS_AUDIT_STATUS, TRUSTED_LAPTOP_ACCESS_AUDIT_TARGET,
        build_reborn_runtime,
    };

    const RUNTIME_POLL_TIMEOUT: Duration = Duration::from_secs(10);
    const RUNTIME_SEND_TIMEOUT: Duration = Duration::from_secs(15);

    async fn stop_turn_runner_worker_for_manual_state_test(runtime: &super::RebornRuntime) {
        runtime.turn_scheduler.stop_for_test().await;
    }

    fn local_dev_runtime_policy() -> EffectiveRuntimePolicy {
        EffectiveRuntimePolicy {
            deployment: DeploymentMode::LocalSingleUser,
            requested_profile: RuntimeProfile::LocalDev,
            resolved_profile: RuntimeProfile::LocalDev,
            filesystem_backend: FilesystemBackendKind::HostWorkspace,
            process_backend: ProcessBackendKind::LocalHost,
            network_mode: NetworkMode::DirectLogged,
            secret_mode: SecretMode::ScrubbedEnv,
            approval_policy: ApprovalPolicy::AskDestructive,
            audit_mode: AuditMode::LocalMinimal,
        }
    }

    #[derive(Debug)]
    struct RecordingGateway {
        reply: String,
        requests: Arc<StdMutex<Vec<HostManagedModelRequest>>>,
    }

    #[derive(Debug, Default)]
    struct FailingSkillContextSource {
        calls: AtomicUsize,
    }

    #[derive(Debug, Default)]
    struct ToolCallingGateway {
        calls: StdMutex<usize>,
        stream_model_calls: StdMutex<usize>,
        requests: StdMutex<Vec<HostManagedModelRequest>>,
    }

    #[derive(Debug, Default)]
    struct AuthGateToolCallingGateway {
        requests: StdMutex<Vec<HostManagedModelRequest>>,
    }

    #[derive(Debug, Default)]
    struct WorkspaceListingGateway {
        calls: StdMutex<usize>,
        requests: StdMutex<Vec<HostManagedModelRequest>>,
    }

    struct StaticSkillContextSource {
        candidates: Vec<HostSkillContextCandidate>,
    }

    #[derive(Debug)]
    struct AllowingTriggerFireAccessChecker;

    impl StaticSkillContextSource {
        fn new(candidates: Vec<HostSkillContextCandidate>) -> Self {
            Self { candidates }
        }
    }

    #[async_trait]
    impl TriggerFireAccessChecker for AllowingTriggerFireAccessChecker {
        async fn check_trigger_fire_access(
            &self,
            _request: TriggerFireAccessCheck,
        ) -> Result<TriggerFireAccessDecision, TriggerFireAccessError> {
            Ok(TriggerFireAccessDecision::Allowed)
        }
    }

    #[async_trait]
    impl HostSkillContextSource for StaticSkillContextSource {
        async fn load_skill_context_candidates(
            &self,
            _run_context: &LoopRunContext,
        ) -> Result<Vec<HostSkillContextCandidate>, HostSkillContextBuildError> {
            Ok(self.candidates.clone())
        }
    }

    #[async_trait]
    impl HostManagedModelGateway for RecordingGateway {
        async fn stream_model(
            &self,
            request: HostManagedModelRequest,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            self.requests
                .lock()
                .expect("recording gateway requests lock poisoned")
                .push(request);
            Ok(HostManagedModelResponse::assistant_reply(
                self.reply.clone(),
            ))
        }
    }

    #[async_trait]
    impl HostSkillContextSource for FailingSkillContextSource {
        async fn load_skill_context_candidates(
            &self,
            _run_context: &LoopRunContext,
        ) -> Result<Vec<HostSkillContextCandidate>, HostSkillContextBuildError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(HostSkillContextBuildError::SourceUnavailable)
        }
    }

    #[async_trait]
    impl HostManagedModelGateway for ToolCallingGateway {
        async fn stream_model(
            &self,
            request: HostManagedModelRequest,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            *self
                .stream_model_calls
                .lock()
                .expect("tool gateway stream count lock poisoned") += 1;
            self.requests
                .lock()
                .expect("tool gateway requests lock poisoned")
                .push(request);
            Err(HostManagedModelError::safe(
                HostManagedModelErrorKind::InvalidRequest,
                "expected capability-aware model path",
            ))
        }

        async fn stream_model_with_capabilities(
            &self,
            request: HostManagedModelRequest,
            capabilities: Arc<dyn LoopCapabilityPort>,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            let call_index = {
                let mut calls = self.calls.lock().expect("tool gateway lock poisoned");
                let call_index = *calls;
                *calls += 1;
                call_index
            };
            self.requests
                .lock()
                .expect("tool gateway requests lock poisoned")
                .push(request.clone());
            if call_index > 0 {
                let tool_result = request
                    .messages
                    .iter()
                    .find(|message| message.role == HostManagedModelMessageRole::ToolResult)
                    .expect("second model call should include tool result");
                assert!(
                    tool_result.content.contains("hello from tool"),
                    "tool result should expose hydrated capability output, got {}",
                    tool_result.content
                );
                let provider_call = tool_result
                    .tool_result_provider_call
                    .as_ref()
                    .expect("provider replay metadata");
                assert_eq!(provider_call.provider_call_id, "call-1");
                assert_eq!(
                    provider_call.capability_id,
                    CapabilityId::new("builtin.echo").unwrap()
                );
                return Ok(HostManagedModelResponse::assistant_reply("tool ok"));
            }

            let surface = capabilities
                .visible_capabilities(VisibleCapabilityRequest)
                .await
                .map_err(model_capability_error)?;
            let echo_id = CapabilityId::new("builtin.echo").expect("echo id");
            assert!(
                surface
                    .descriptors
                    .iter()
                    .any(|descriptor| descriptor.capability_id == echo_id),
                "builtin echo must be visible through local-dev runtime capability surface"
            );
            let echo_tool = capabilities
                .tool_definitions()
                .map_err(model_capability_error)?
                .into_iter()
                .find(|definition| definition.capability_id == echo_id)
                .expect("echo provider tool definition");
            let candidate = capabilities
                .register_provider_tool_call(ProviderToolCall {
                    provider_id: "test-provider".to_string(),
                    provider_model_id: "test-model".to_string(),
                    turn_id: Some("provider-turn-1".to_string()),
                    id: "call-1".to_string(),
                    name: echo_tool.name,
                    arguments: serde_json::json!({"message": "hello from tool"}),
                    response_reasoning: None,
                    reasoning: None,
                    signature: None,
                })
                .await
                .map_err(model_capability_error)?;
            Ok(HostManagedModelResponse::capability_calls(
                vec![candidate],
                "",
            ))
        }
    }

    /// A long echo argument (well over the safe-preview 512-byte string cap) so
    /// the default-observer test can prove the payload is truncated before the
    /// observer sees it.
    const LARGE_ECHO_MESSAGE: &str = "PAYLOAD0123456789ABCDEF_";

    #[derive(Debug, Default)]
    struct LargeEchoToolCallingGateway {
        calls: StdMutex<usize>,
    }

    #[async_trait]
    impl HostManagedModelGateway for LargeEchoToolCallingGateway {
        async fn stream_model(
            &self,
            _request: HostManagedModelRequest,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            Err(HostManagedModelError::safe(
                HostManagedModelErrorKind::InvalidRequest,
                "expected capability-aware model path",
            ))
        }

        async fn stream_model_with_capabilities(
            &self,
            _request: HostManagedModelRequest,
            capabilities: Arc<dyn LoopCapabilityPort>,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            let call_index = {
                let mut calls = self.calls.lock().expect("large echo gateway lock poisoned");
                let call_index = *calls;
                *calls += 1;
                call_index
            };
            if call_index > 0 {
                return Ok(HostManagedModelResponse::assistant_reply("tool ok"));
            }
            let echo_id = CapabilityId::new("builtin.echo").expect("echo id");
            let echo_tool = capabilities
                .tool_definitions()
                .map_err(model_capability_error)?
                .into_iter()
                .find(|definition| definition.capability_id == echo_id)
                .expect("echo provider tool definition");
            // ~2.4 KB message: far over the 512-byte string preview cap.
            let big_message = LARGE_ECHO_MESSAGE.repeat(100);
            let candidate = capabilities
                .register_provider_tool_call(ProviderToolCall {
                    provider_id: "test-provider".to_string(),
                    provider_model_id: "test-model".to_string(),
                    turn_id: Some("provider-turn-1".to_string()),
                    id: "call-1".to_string(),
                    name: echo_tool.name,
                    arguments: serde_json::json!({ "message": big_message }),
                    response_reasoning: None,
                    reasoning: None,
                    signature: None,
                })
                .await
                .map_err(model_capability_error)?;
            Ok(HostManagedModelResponse::capability_calls(
                vec![candidate],
                "",
            ))
        }
    }

    #[async_trait]
    impl HostManagedModelGateway for AuthGateToolCallingGateway {
        async fn stream_model(
            &self,
            request: HostManagedModelRequest,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            self.requests
                .lock()
                .expect("auth-gate gateway requests lock poisoned")
                .push(request);
            Err(HostManagedModelError::safe(
                HostManagedModelErrorKind::InvalidRequest,
                "expected capability-aware model path",
            ))
        }

        async fn stream_model_with_capabilities(
            &self,
            request: HostManagedModelRequest,
            capabilities: Arc<dyn LoopCapabilityPort>,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            self.requests
                .lock()
                .expect("auth-gate gateway requests lock poisoned")
                .push(request);
            let notion_search_id =
                CapabilityId::new("notion.notion-search").expect("notion search id");
            let notion_tool = capabilities
                .tool_definitions()
                .map_err(model_capability_error)?
                .into_iter()
                .find(|definition| definition.capability_id == notion_search_id)
                .expect("activated Notion capability should be visible");
            let candidate = capabilities
                .register_provider_tool_call(ProviderToolCall {
                    provider_id: "test-provider".to_string(),
                    provider_model_id: "test-model".to_string(),
                    turn_id: Some("provider-turn-auth-gate".to_string()),
                    id: "call-auth-gate".to_string(),
                    name: notion_tool.name,
                    arguments: serde_json::json!({ "query": "project notes" }),
                    response_reasoning: None,
                    reasoning: None,
                    signature: None,
                })
                .await
                .map_err(model_capability_error)?;
            Ok(HostManagedModelResponse::capability_calls(
                vec![candidate],
                "",
            ))
        }
    }

    #[async_trait]
    impl HostManagedModelGateway for WorkspaceListingGateway {
        async fn stream_model(
            &self,
            request: HostManagedModelRequest,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            self.requests
                .lock()
                .expect("workspace gateway requests lock poisoned")
                .push(request);
            Err(HostManagedModelError::safe(
                HostManagedModelErrorKind::InvalidRequest,
                "expected capability-aware model path",
            ))
        }

        async fn stream_model_with_capabilities(
            &self,
            request: HostManagedModelRequest,
            capabilities: Arc<dyn LoopCapabilityPort>,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            let call_index = {
                let mut calls = self.calls.lock().expect("workspace gateway lock poisoned");
                let call_index = *calls;
                *calls += 1;
                call_index
            };
            self.requests
                .lock()
                .expect("workspace gateway requests lock poisoned")
                .push(request.clone());
            if call_index > 0 {
                let tool_result = request
                    .messages
                    .iter()
                    .find(|message| message.role == HostManagedModelMessageRole::ToolResult)
                    .expect("second model call should include tool result");
                assert!(
                    tool_result.content.contains("workspace-sentinel.txt"),
                    "workspace listing should expose configured workspace root, got {}",
                    tool_result.content
                );
                return Ok(HostManagedModelResponse::assistant_reply("workspace ok"));
            }

            let list_dir_id = CapabilityId::new("builtin.list_dir").expect("list_dir id");
            let list_dir_tool = capabilities
                .tool_definitions()
                .map_err(model_capability_error)?
                .into_iter()
                .find(|definition| definition.capability_id == list_dir_id)
                .expect("list_dir provider tool definition");
            let candidate = capabilities
                .register_provider_tool_call(ProviderToolCall {
                    provider_id: "test-provider".to_string(),
                    provider_model_id: "test-model".to_string(),
                    turn_id: Some("provider-turn-1".to_string()),
                    id: "call-1".to_string(),
                    name: list_dir_tool.name,
                    arguments: serde_json::json!({"path": "/workspace"}),
                    response_reasoning: None,
                    reasoning: None,
                    signature: None,
                })
                .await
                .map_err(model_capability_error)?;
            Ok(HostManagedModelResponse::capability_calls(
                vec![candidate],
                "",
            ))
        }
    }

    fn model_capability_error(error: impl std::fmt::Display) -> HostManagedModelError {
        let safe_summary = error.to_string();
        HostManagedModelError::safe(HostManagedModelErrorKind::Unavailable, safe_summary)
    }

    #[cfg(feature = "root-llm-provider")]
    struct RuntimeEnvGuard {
        name: &'static str,
        previous: Option<String>,
    }

    #[cfg(feature = "root-llm-provider")]
    impl RuntimeEnvGuard {
        fn set(name: &'static str, value: &str) -> Self {
            let previous = ironclaw_common::env_helpers::env_or_override(name);
            ironclaw_common::env_helpers::set_runtime_env(name, value);
            Self { name, previous }
        }

        fn unset(name: &'static str) -> Self {
            let previous = ironclaw_common::env_helpers::env_or_override(name);
            ironclaw_common::env_helpers::remove_runtime_env(name);
            Self { name, previous }
        }
    }

    #[cfg(feature = "root-llm-provider")]
    impl Drop for RuntimeEnvGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => ironclaw_common::env_helpers::set_runtime_env(self.name, value),
                None => ironclaw_common::env_helpers::remove_runtime_env(self.name),
            }
        }
    }

    #[cfg(feature = "root-llm-provider")]
    async fn start_nearai_auth_capture_server() -> (String, tokio::sync::oneshot::Receiver<String>)
    {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpSocket;

        let socket = TcpSocket::new_v4().expect("test server socket");
        socket
            .bind("127.0.0.1:0".parse().expect("test server address"))
            .expect("test server binds");
        let listener = socket.listen(1024).expect("test server listens");
        let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
        let (auth_tx, auth_rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            let mut auth_tx = Some(auth_tx);
            loop {
                let (mut stream, _) = listener.accept().await.expect("accept test request");
                let mut buffer = Vec::new();
                loop {
                    let mut chunk = [0_u8; 1024];
                    let read = stream.read(&mut chunk).await.expect("read test request");
                    if read == 0 {
                        break;
                    }
                    buffer.extend_from_slice(&chunk[..read]);
                    if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }

                let request = String::from_utf8_lossy(&buffer);
                let request_line = request.lines().next().unwrap_or_default();
                let auth_header = request
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                    .map(|(_, value)| value.trim())
                    .unwrap_or_default()
                    .to_string();
                let is_chat_completion = request_line.contains("/v1/chat/completions");
                let body = if is_chat_completion {
                    r#"{"choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#
                } else {
                    r#"{"data":[]}"#
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("write test response");

                if is_chat_completion {
                    if let Some(auth_tx) = auth_tx.take() {
                        let _ = auth_tx.send(auth_header);
                    }
                    break;
                }
            }
        });

        (base_url, auth_rx)
    }

    #[cfg(feature = "root-llm-provider")]
    fn nearai_gateway_test_request() -> HostManagedModelRequest {
        HostManagedModelRequest {
            model_profile_id: ironclaw_turns::run_profile::ModelProfileId::new("interactive_model")
                .expect("model profile id"),
            messages: vec![ironclaw_loop_support::HostManagedModelMessage {
                role: HostManagedModelMessageRole::User,
                content: "hello model".to_string(),
                content_ref: ironclaw_turns::LoopMessageRef::new(
                    "msg:22222222-2222-2222-2222-222222222222",
                )
                .expect("message ref"),
                tool_result_provider_call: None,
                tool_result_content: None,
                image_parts: Vec::new(),
            }],
            surface_version: None,
            resolved_model_route: None,
            run_id: TurnRunId::new(),
            turn_id: TurnId::new(),
        }
    }

    #[cfg(feature = "root-llm-provider")]
    #[derive(Debug)]
    struct RecordingLlmProvider {
        active_model: StdMutex<String>,
        requests: StdMutex<Vec<Option<String>>>,
    }

    #[cfg(feature = "root-llm-provider")]
    impl RecordingLlmProvider {
        fn new(active_model: &str) -> Self {
            Self {
                active_model: StdMutex::new(active_model.to_string()),
                requests: StdMutex::new(Vec::new()),
            }
        }
    }

    #[cfg(feature = "root-llm-provider")]
    #[async_trait]
    impl ironclaw_llm::LlmProvider for RecordingLlmProvider {
        fn model_name(&self) -> &str {
            "recording-provider"
        }

        fn cost_per_token(&self) -> (rust_decimal::Decimal, rust_decimal::Decimal) {
            (rust_decimal::Decimal::ZERO, rust_decimal::Decimal::ZERO)
        }

        async fn complete(
            &self,
            request: ironclaw_llm::CompletionRequest,
        ) -> Result<ironclaw_llm::CompletionResponse, ironclaw_llm::LlmError> {
            self.requests
                .lock()
                .expect("recording provider request lock poisoned")
                .push(request.model);
            Ok(ironclaw_llm::CompletionResponse {
                content: "ok".to_string(),
                input_tokens: 1,
                output_tokens: 1,
                finish_reason: ironclaw_llm::FinishReason::Stop,
                reasoning: None,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
        }

        async fn complete_with_tools(
            &self,
            request: ironclaw_llm::ToolCompletionRequest,
        ) -> Result<ironclaw_llm::ToolCompletionResponse, ironclaw_llm::LlmError> {
            self.requests
                .lock()
                .expect("recording provider request lock poisoned")
                .push(request.model);
            Ok(ironclaw_llm::ToolCompletionResponse {
                content: Some("ok".to_string()),
                tool_calls: Vec::new(),
                input_tokens: 1,
                output_tokens: 1,
                finish_reason: ironclaw_llm::FinishReason::Stop,
                reasoning: None,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
        }

        fn active_model_name(&self) -> String {
            self.active_model
                .lock()
                .expect("recording provider active-model lock poisoned")
                .clone()
        }

        fn set_model(&self, model: &str) -> Result<(), ironclaw_llm::LlmError> {
            *self
                .active_model
                .lock()
                .expect("recording provider active-model lock poisoned") = model.to_string();
            Ok(())
        }
    }

    #[cfg(feature = "root-llm-provider")]
    #[tokio::test]
    async fn swappable_gateway_uses_current_active_model_for_requests() {
        let provider = Arc::new(RecordingLlmProvider::new("boot-model"));
        let raw: Arc<dyn ironclaw_llm::LlmProvider> = provider.clone();
        let session =
            ironclaw_llm::create_session_manager(ironclaw_llm::SessionConfig::default()).await;
        let bundle = super::wrap_swappable_gateway(raw, session, None).expect("gateway bundle");

        bundle
            .gateway
            .stream_model(nearai_gateway_test_request())
            .await
            .expect("first request");
        bundle
            .reload
            .reload_handle
            .primary_provider()
            .set_model("reloaded-model")
            .expect("set active model");
        bundle
            .gateway
            .stream_model(nearai_gateway_test_request())
            .await
            .expect("second request");

        let requests = provider
            .requests
            .lock()
            .expect("recording provider request lock poisoned");
        assert_eq!(
            *requests,
            vec![
                Some("boot-model".to_string()),
                Some("reloaded-model".to_string())
            ],
            "production gateway must not keep sending the model selected at boot"
        );
    }

    fn skill_md(name: &str, description: &str, prompt: &str) -> String {
        format!(
            "---\nname: {name}\ndescription: {description}\nactivation:\n  keywords: [\"{name}\"]\n---\n\n{prompt}"
        )
    }

    fn user_skill_dir(
        storage_root: &std::path::Path,
        tenant_id: &str,
        user_id: &str,
        name: &str,
    ) -> std::path::PathBuf {
        storage_root
            .join("tenants")
            .join(tenant_id)
            .join("users")
            .join(user_id)
            .join("skills")
            .join(name)
    }

    fn skill_md_with_setup_marker(
        name: &str,
        description: &str,
        marker: &str,
        prompt: &str,
    ) -> String {
        format!(
            "---\nname: {name}\ndescription: {description}\nactivation:\n  keywords: [\"{name}\"]\n  setup_marker: \"{marker}\"\n---\n\n{prompt}"
        )
    }

    fn recorded_request_count(requests: &StdMutex<Vec<HostManagedModelRequest>>) -> usize {
        requests
            .lock()
            .expect("recording gateway requests lock poisoned")
            .len()
    }

    #[cfg(feature = "root-llm-provider")]
    #[tokio::test]
    async fn root_llm_gateway_bootstraps_nearai_session_token_from_env() {
        let _token_guard = RuntimeEnvGuard::set("NEARAI_SESSION_TOKEN", "sess_reborn_env_token");
        let session_dir = tempfile::tempdir().expect("session tempdir");
        let (base_url, auth_rx) = start_nearai_auth_capture_server().await;

        let config = ironclaw_llm::LlmConfig {
            backend: "nearai".to_string(),
            session: ironclaw_llm::SessionConfig {
                auth_base_url: base_url.clone(),
                session_path: session_dir.path().join("session.json"),
            },
            nearai: ironclaw_llm::NearAiConfig {
                model: "test-model".to_string(),
                cheap_model: None,
                base_url,
                api_key: None,
                fallback_model: None,
                max_retries: 0,
                circuit_breaker_threshold: None,
                circuit_breaker_recovery_secs: 30,
                response_cache_enabled: false,
                response_cache_ttl_secs: 3600,
                response_cache_max_entries: 1000,
                failover_cooldown_secs: 300,
                failover_cooldown_threshold: 3,
                smart_routing_cascade: false,
            },
            provider: None,
            bedrock: None,
            gemini_oauth: None,
            openai_codex: None,
            request_timeout_secs: 5,
            cheap_model: None,
            smart_routing_cascade: false,
            max_retries: 0,
            circuit_breaker_threshold: None,
            circuit_breaker_recovery_secs: 30,
            response_cache_enabled: false,
            response_cache_ttl_secs: 3600,
            response_cache_max_entries: 1000,
        };
        let llm = crate::runtime_input::ResolvedRebornLlm::from_llm_config(config);

        let bundle = super::build_llm_gateway(llm).await.expect("gateway builds");
        let response = bundle
            .gateway
            .stream_model(nearai_gateway_test_request())
            .await
            .expect("gateway calls NEAR AI provider");

        assert_eq!(response.safe_text_deltas, vec!["ok".to_string()]);
        let auth_header = tokio::time::timeout(Duration::from_secs(2), auth_rx)
            .await
            .expect("chat request should be captured")
            .expect("auth header should be sent by capture server");
        assert_eq!(auth_header, "Bearer sess_reborn_env_token");
    }

    #[cfg(feature = "root-llm-provider")]
    #[tokio::test]
    async fn runtime_nearai_mcp_bootstraps_from_nearai_session_token() {
        let _token_guard = RuntimeEnvGuard::set("NEARAI_SESSION_TOKEN", "sess_reborn_mcp_token");
        let root = tempfile::tempdir().expect("tempdir");
        let session_dir = tempfile::tempdir().expect("session tempdir");
        let local_dev_root = root.path().join("local-dev");

        let config = ironclaw_llm::LlmConfig {
            backend: "nearai".to_string(),
            session: ironclaw_llm::SessionConfig {
                auth_base_url: "https://private.near.ai".to_string(),
                session_path: session_dir.path().join("session.json"),
            },
            nearai: ironclaw_llm::NearAiConfig {
                model: "test-model".to_string(),
                cheap_model: None,
                base_url: "https://private.near.ai".to_string(),
                api_key: None,
                fallback_model: None,
                max_retries: 0,
                circuit_breaker_threshold: None,
                circuit_breaker_recovery_secs: 30,
                response_cache_enabled: false,
                response_cache_ttl_secs: 3600,
                response_cache_max_entries: 1000,
                failover_cooldown_secs: 300,
                failover_cooldown_threshold: 3,
                smart_routing_cascade: false,
            },
            provider: None,
            bedrock: None,
            gemini_oauth: None,
            openai_codex: None,
            request_timeout_secs: 5,
            cheap_model: None,
            smart_routing_cascade: false,
            max_retries: 0,
            circuit_breaker_threshold: None,
            circuit_breaker_recovery_secs: 30,
            response_cache_enabled: false,
            response_cache_ttl_secs: 3600,
            response_cache_max_entries: 1000,
        };
        let llm = crate::runtime_input::ResolvedRebornLlm::from_llm_config(config);

        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-nearai-session-mcp-owner", local_dev_root)
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_resolved_llm(llm)
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-nearai-session-mcp-tenant".to_string(),
            agent_id: "runtime-nearai-session-mcp-agent".to_string(),
            source_binding_id: "runtime-nearai-session-mcp-source".to_string(),
            reply_target_binding_id: "runtime-nearai-session-mcp-reply".to_string(),
        });

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let local_runtime = runtime
            .services()
            .local_runtime
            .as_ref()
            .expect("local runtime");
        let extension_management = local_runtime
            .extension_management
            .as_ref()
            .expect("extension management");
        let nearai_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "nearai").expect("valid ref");
        let projection = extension_management
            .project(nearai_ref)
            .await
            .expect("NEAR AI MCP projected");
        assert_eq!(projection.phase, LifecyclePhase::Active);

        let capabilities = extension_management
            .active_model_visible_capabilities()
            .await
            .expect("active capabilities");
        assert!(
            capabilities
                .iter()
                .any(|capability| capability.id.as_str() == "nearai.web_search"),
            "nearai.web_search should be active with NEAR AI session-token config"
        );
        stop_turn_runner_worker_for_manual_state_test(&runtime).await;
    }

    #[cfg(all(feature = "root-llm-provider", feature = "libsql"))]
    #[tokio::test]
    async fn runtime_nearai_mcp_bootstraps_from_stored_nearai_api_key() {
        let _session_token_guard = RuntimeEnvGuard::unset("NEARAI_SESSION_TOKEN");
        let _api_key_guard = RuntimeEnvGuard::unset("NEARAI_API_KEY");
        let root = tempfile::tempdir().expect("tempdir");
        let local_dev_root = root.path().join("local-dev");
        let session_dir = tempfile::tempdir().expect("session tempdir");

        let services = crate::build_reborn_services(
            RebornBuildInput::local_dev("runtime-nearai-stored-mcp-owner", local_dev_root.clone())
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .await
        .expect("services build for stored key seed");
        crate::LlmKeyStore::new(services.secret_store())
            .put(
                "nearai",
                ironclaw_secrets::SecretMaterial::from("sk-reborn-stored-nearai-mcp-key"),
            )
            .await
            .expect("stored key seeded");
        drop(services);

        let config = ironclaw_llm::LlmConfig {
            backend: "nearai".to_string(),
            session: ironclaw_llm::SessionConfig {
                auth_base_url: "https://private.near.ai".to_string(),
                session_path: session_dir.path().join("session.json"),
            },
            nearai: ironclaw_llm::NearAiConfig {
                model: "test-model".to_string(),
                cheap_model: None,
                base_url: "https://cloud-api.near.ai".to_string(),
                api_key: None,
                fallback_model: None,
                max_retries: 0,
                circuit_breaker_threshold: None,
                circuit_breaker_recovery_secs: 30,
                response_cache_enabled: false,
                response_cache_ttl_secs: 3600,
                response_cache_max_entries: 1000,
                failover_cooldown_secs: 300,
                failover_cooldown_threshold: 3,
                smart_routing_cascade: false,
            },
            provider: None,
            bedrock: None,
            gemini_oauth: None,
            openai_codex: None,
            request_timeout_secs: 5,
            cheap_model: None,
            smart_routing_cascade: false,
            max_retries: 0,
            circuit_breaker_threshold: None,
            circuit_breaker_recovery_secs: 30,
            response_cache_enabled: false,
            response_cache_ttl_secs: 3600,
            response_cache_max_entries: 1000,
        };
        let llm = crate::runtime_input::ResolvedRebornLlm::from_llm_config(config);

        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-nearai-stored-mcp-owner", local_dev_root)
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_resolved_llm(llm)
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-nearai-stored-mcp-tenant".to_string(),
            agent_id: "runtime-nearai-stored-mcp-agent".to_string(),
            source_binding_id: "runtime-nearai-stored-mcp-source".to_string(),
            reply_target_binding_id: "runtime-nearai-stored-mcp-reply".to_string(),
        });

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let local_runtime = runtime
            .services()
            .local_runtime
            .as_ref()
            .expect("local runtime");
        let extension_management = local_runtime
            .extension_management
            .as_ref()
            .expect("extension management");
        let nearai_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "nearai").expect("valid ref");
        let projection = extension_management
            .project(nearai_ref)
            .await
            .expect("NEAR AI MCP projected");
        assert_eq!(projection.phase, LifecyclePhase::Active);

        let capabilities = extension_management
            .active_model_visible_capabilities()
            .await
            .expect("active capabilities");
        assert!(
            capabilities
                .iter()
                .any(|capability| capability.id.as_str() == "nearai.web_search"),
            "nearai.web_search should be active with stored NEAR AI API key config"
        );
        stop_turn_runner_worker_for_manual_state_test(&runtime).await;
    }

    #[cfg(all(feature = "root-llm-provider", feature = "libsql"))]
    async fn nearai_mcp_runtime_access_secret(
        runtime: &super::RebornRuntime,
        owner_scope: ResourceScope,
    ) -> String {
        let product_auth = runtime
            .services()
            .product_auth
            .as_ref()
            .expect("product auth");
        let auth_scope = ironclaw_auth::AuthProductScope::credential_owner(
            &owner_scope,
            ironclaw_auth::AuthSurface::Api,
        );
        let accounts = product_auth
            .credential_account_record_source()
            .accounts_for_owner(&auth_scope)
            .await
            .expect("NEAR AI product-auth accounts");
        let account = accounts
            .into_iter()
            .find(|account| {
                account.provider.as_str() == "nearai"
                    && account.status == ironclaw_auth::CredentialAccountStatus::Configured
            })
            .expect("configured NEAR AI product-auth account");

        assert_eq!(account.scope.resource.tenant_id, owner_scope.tenant_id);
        assert_eq!(account.scope.resource.user_id, owner_scope.user_id);
        assert_eq!(account.scope.resource.agent_id, owner_scope.agent_id);
        assert_eq!(account.scope.resource.project_id, owner_scope.project_id);

        let handle = account.access_secret.expect("NEAR AI access secret");
        let store = runtime.services().secret_store();
        let lease = store
            .lease_once(&account.scope.resource, &handle)
            .await
            .expect("NEAR AI access secret lease");
        let material = store
            .consume(&account.scope.resource, lease.id)
            .await
            .expect("NEAR AI access secret material");
        secrecy::ExposeSecret::expose_secret(&material).to_string()
    }

    #[cfg(all(feature = "root-llm-provider", feature = "libsql"))]
    #[tokio::test]
    async fn runtime_nearai_mcp_prebuild_api_key_is_not_replaced_by_stored_key() {
        let _session_token_guard = RuntimeEnvGuard::unset("NEARAI_SESSION_TOKEN");
        let _api_key_guard = RuntimeEnvGuard::unset("NEARAI_API_KEY");
        let root = tempfile::tempdir().expect("tempdir");
        let local_dev_root = root.path().join("local-dev");
        let session_dir = tempfile::tempdir().expect("session tempdir");
        let owner = "runtime-nearai-prebuild-mcp-owner";
        let tenant = "runtime-nearai-prebuild-mcp-tenant";
        let agent = "runtime-nearai-prebuild-mcp-agent";

        let services = crate::build_reborn_services(
            RebornBuildInput::local_dev(owner, local_dev_root.clone())
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .await
        .expect("services build for stored key seed");
        crate::LlmKeyStore::new(services.secret_store())
            .put(
                "nearai",
                ironclaw_secrets::SecretMaterial::from("sk-post-build-stored-nearai-mcp-key"),
            )
            .await
            .expect("stored key seeded");
        drop(services);

        let config = ironclaw_llm::LlmConfig {
            backend: "nearai".to_string(),
            session: ironclaw_llm::SessionConfig {
                auth_base_url: "https://private.near.ai".to_string(),
                session_path: session_dir.path().join("session.json"),
            },
            nearai: ironclaw_llm::NearAiConfig {
                model: "test-model".to_string(),
                cheap_model: None,
                base_url: "https://cloud-api.near.ai".to_string(),
                api_key: Some(secrecy::SecretString::from("sk-prebuild-nearai-mcp-key")),
                fallback_model: None,
                max_retries: 0,
                circuit_breaker_threshold: None,
                circuit_breaker_recovery_secs: 30,
                response_cache_enabled: false,
                response_cache_ttl_secs: 3600,
                response_cache_max_entries: 1000,
                failover_cooldown_secs: 300,
                failover_cooldown_threshold: 3,
                smart_routing_cascade: false,
            },
            provider: None,
            bedrock: None,
            gemini_oauth: None,
            openai_codex: None,
            request_timeout_secs: 5,
            cheap_model: None,
            smart_routing_cascade: false,
            max_retries: 0,
            circuit_breaker_threshold: None,
            circuit_breaker_recovery_secs: 30,
            response_cache_enabled: false,
            response_cache_ttl_secs: 3600,
            response_cache_max_entries: 1000,
        };
        let llm = crate::runtime_input::ResolvedRebornLlm::from_llm_config(config);

        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(owner, local_dev_root)
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_resolved_llm(llm)
        .with_identity(RebornRuntimeIdentity {
            tenant_id: tenant.to_string(),
            agent_id: agent.to_string(),
            source_binding_id: "runtime-nearai-prebuild-mcp-source".to_string(),
            reply_target_binding_id: "runtime-nearai-prebuild-mcp-reply".to_string(),
        });

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let owner_scope = ResourceScope {
            tenant_id: TenantId::new(tenant).expect("tenant"),
            user_id: UserId::new(owner).expect("owner"),
            agent_id: Some(AgentId::new(agent).expect("agent")),
            project_id: None::<ProjectId>,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        let material = nearai_mcp_runtime_access_secret(&runtime, owner_scope).await;

        assert_eq!(material, "sk-prebuild-nearai-mcp-key");
        stop_turn_runner_worker_for_manual_state_test(&runtime).await;
    }

    /// Counts how many times the runtime drives this provider and answers with a
    /// fixed sentinel, so a test can prove an injected provider — not one built
    /// from config — is the one the gateway actually calls.
    #[cfg(feature = "root-llm-provider")]
    struct CountingOverrideProvider {
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[cfg(feature = "root-llm-provider")]
    #[async_trait::async_trait]
    impl ironclaw_llm::LlmProvider for CountingOverrideProvider {
        fn model_name(&self) -> &str {
            "mock-override-model"
        }

        fn cost_per_token(&self) -> (rust_decimal::Decimal, rust_decimal::Decimal) {
            (rust_decimal::Decimal::ZERO, rust_decimal::Decimal::ZERO)
        }

        async fn complete(
            &self,
            _request: ironclaw_llm::CompletionRequest,
        ) -> Result<ironclaw_llm::CompletionResponse, ironclaw_llm::LlmError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(ironclaw_llm::CompletionResponse {
                content: "override-driven".to_string(),
                input_tokens: 0,
                output_tokens: 0,
                finish_reason: ironclaw_llm::FinishReason::Stop,
                reasoning: None,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
        }

        async fn complete_with_tools(
            &self,
            _request: ironclaw_llm::ToolCompletionRequest,
        ) -> Result<ironclaw_llm::ToolCompletionResponse, ironclaw_llm::LlmError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(ironclaw_llm::ToolCompletionResponse {
                content: Some("override-driven".to_string()),
                tool_calls: Vec::new(),
                input_tokens: 0,
                output_tokens: 0,
                finish_reason: ironclaw_llm::FinishReason::Stop,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
                reasoning: None,
            })
        }
    }

    /// The LLM-provider-instrumentation seam: when a caller installs a factory
    /// via `ResolvedRebornLlm::with_provider_factory` (how the bench wraps an
    /// instrumented provider to capture reasoning / tokens / cost / system-prompt
    /// / tool definitions), the gateway must drive the factory's output. Here the
    /// factory ignores the config-built provider and returns a counting mock, so
    /// if the factory were not applied the gateway would drive the config-built
    /// provider (dead endpoint) instead of returning the mock's sentinel.
    #[cfg(feature = "root-llm-provider")]
    #[tokio::test]
    async fn build_llm_gateway_applies_provider_factory() {
        let session_dir = tempfile::tempdir().expect("session tempdir");
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mock: Arc<dyn ironclaw_llm::LlmProvider> = Arc::new(CountingOverrideProvider {
            calls: Arc::clone(&calls),
        });

        let config = ironclaw_llm::LlmConfig {
            backend: "nearai".to_string(),
            session: ironclaw_llm::SessionConfig {
                auth_base_url: "http://127.0.0.1:1".to_string(),
                session_path: session_dir.path().join("session.json"),
            },
            nearai: ironclaw_llm::NearAiConfig {
                model: "config-model-should-not-be-used".to_string(),
                cheap_model: None,
                base_url: "http://127.0.0.1:1".to_string(),
                api_key: None,
                fallback_model: None,
                max_retries: 0,
                circuit_breaker_threshold: None,
                circuit_breaker_recovery_secs: 30,
                response_cache_enabled: false,
                response_cache_ttl_secs: 3600,
                response_cache_max_entries: 1000,
                failover_cooldown_secs: 300,
                failover_cooldown_threshold: 3,
                smart_routing_cascade: false,
            },
            provider: None,
            bedrock: None,
            gemini_oauth: None,
            openai_codex: None,
            request_timeout_secs: 5,
            cheap_model: None,
            smart_routing_cascade: false,
            max_retries: 0,
            circuit_breaker_threshold: None,
            circuit_breaker_recovery_secs: 30,
            response_cache_enabled: false,
            response_cache_ttl_secs: 3600,
            response_cache_max_entries: 1000,
        };

        let factory_mock = Arc::clone(&mock);
        let llm = crate::runtime_input::ResolvedRebornLlm::from_llm_config(config)
            .with_provider_factory(Arc::new(move |_built| Arc::clone(&factory_mock)));
        let bundle = super::build_llm_gateway(llm)
            .await
            .expect("gateway builds with the provider factory");

        let response = bundle
            .gateway
            .stream_model(nearai_gateway_test_request())
            .await
            .expect("gateway drives the factory-produced provider");

        assert_eq!(
            response.safe_text_deltas,
            vec!["override-driven".to_string()],
            "gateway must return the factory provider's response, not the config-built one"
        );
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the override provider should be invoked exactly once"
        );
    }

    /// Provider wrapper that counts model calls and delegates to its inner — a
    /// stand-in for the bench's instrumentation wrapper. Unlike
    /// `CountingOverrideProvider`, it wraps `inner` so swapping the inner (via a
    /// live reload of a `SwappableLlmProvider`) is observable through it.
    #[cfg(feature = "root-llm-provider")]
    struct CountingWrapperProvider {
        inner: Arc<dyn ironclaw_llm::LlmProvider>,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[cfg(feature = "root-llm-provider")]
    #[async_trait::async_trait]
    impl ironclaw_llm::LlmProvider for CountingWrapperProvider {
        fn model_name(&self) -> &str {
            self.inner.model_name()
        }

        fn cost_per_token(&self) -> (rust_decimal::Decimal, rust_decimal::Decimal) {
            self.inner.cost_per_token()
        }

        async fn complete(
            &self,
            request: ironclaw_llm::CompletionRequest,
        ) -> Result<ironclaw_llm::CompletionResponse, ironclaw_llm::LlmError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.complete(request).await
        }

        async fn complete_with_tools(
            &self,
            request: ironclaw_llm::ToolCompletionRequest,
        ) -> Result<ironclaw_llm::ToolCompletionResponse, ironclaw_llm::LlmError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.complete_with_tools(request).await
        }
    }

    /// Minimal nearai `LlmConfig` pointed at a dead endpoint: it *builds* lazily
    /// (no connection at construction) but any model call errors. Enough to
    /// exercise gateway/reload wiring without a network.
    #[cfg(feature = "root-llm-provider")]
    fn dead_endpoint_nearai_config(session_path: std::path::PathBuf) -> ironclaw_llm::LlmConfig {
        ironclaw_llm::LlmConfig {
            backend: "nearai".to_string(),
            session: ironclaw_llm::SessionConfig {
                auth_base_url: "http://127.0.0.1:1".to_string(),
                session_path,
            },
            nearai: ironclaw_llm::NearAiConfig {
                model: "config-model".to_string(),
                cheap_model: None,
                base_url: "http://127.0.0.1:1".to_string(),
                api_key: None,
                fallback_model: None,
                max_retries: 0,
                circuit_breaker_threshold: None,
                circuit_breaker_recovery_secs: 30,
                response_cache_enabled: false,
                response_cache_ttl_secs: 3600,
                response_cache_max_entries: 1000,
                failover_cooldown_secs: 300,
                failover_cooldown_threshold: 3,
                smart_routing_cascade: false,
            },
            provider: None,
            bedrock: None,
            gemini_oauth: None,
            openai_codex: None,
            request_timeout_secs: 5,
            cheap_model: None,
            smart_routing_cascade: false,
            max_retries: 0,
            circuit_breaker_threshold: None,
            circuit_breaker_recovery_secs: 30,
            response_cache_enabled: false,
            response_cache_ttl_secs: 3600,
            response_cache_max_entries: 1000,
        }
    }

    /// Regression guard for Firat's review: the provider factory (caller
    /// instrumentation) must survive a live config reload. `build_llm_gateway`
    /// wraps the factory over the `SwappableLlmProvider`, so reloading — which
    /// swaps the swappable's *inner* — keeps the wrapper in the call path. If the
    /// factory were applied to the bare provider instead, the first reload would
    /// silently drop instrumentation and this test's post-reload count would stay
    /// at 1.
    #[cfg(feature = "root-llm-provider")]
    #[tokio::test]
    async fn provider_factory_survives_live_reload() {
        let session_dir = tempfile::tempdir().expect("session tempdir");
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls_for_factory = Arc::clone(&calls);
        let factory: crate::runtime_input::RebornProviderFactory = Arc::new(move |inner| {
            Arc::new(CountingWrapperProvider {
                inner,
                calls: Arc::clone(&calls_for_factory),
            }) as Arc<dyn ironclaw_llm::LlmProvider>
        });

        let config = dead_endpoint_nearai_config(session_dir.path().join("session.json"));
        let llm = crate::runtime_input::ResolvedRebornLlm::from_llm_config(config.clone())
            .with_provider_factory(factory);
        let bundle = super::build_llm_gateway(llm)
            .await
            .expect("gateway builds with the provider factory");

        // First model call routes through the instrumentation wrapper. The dead
        // endpoint makes the underlying call error, but the wrapper counts before
        // delegating, so the result is irrelevant — only that it was observed.
        let _ = bundle
            .gateway
            .stream_model(nearai_gateway_test_request())
            .await;
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the instrumentation wrapper should observe the first model call"
        );

        // Live config reload: rebuild the chain and atomically swap the
        // swappable's inner provider — exactly what the WebUI settings path does.
        bundle
            .reload
            .reload_handle
            .reload(&config, Arc::clone(&bundle.reload.session))
            .await
            .expect("live reload rebuilds the provider chain");

        let _ = bundle
            .gateway
            .stream_model(nearai_gateway_test_request())
            .await;
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "the instrumentation wrapper must still observe model calls after a live reload"
        );
    }

    #[cfg(all(feature = "root-llm-provider", feature = "libsql"))]
    #[tokio::test]
    async fn local_dev_runtime_startup_uses_stored_nearai_api_key_after_restart() {
        let _session_token_guard = RuntimeEnvGuard::unset("NEARAI_SESSION_TOKEN");
        let _api_key_guard = RuntimeEnvGuard::unset("NEARAI_API_KEY");
        let root = tempfile::tempdir().expect("tempdir");
        let local_dev_root = root.path().join("local-dev");
        let session_dir = tempfile::tempdir().expect("session tempdir");
        let (base_url, auth_rx) = start_nearai_auth_capture_server().await;

        let services = crate::build_reborn_services(
            RebornBuildInput::local_dev("runtime-nearai-stored-key-owner", local_dev_root.clone())
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .await
        .expect("services build for stored key seed");
        crate::LlmKeyStore::new(services.secret_store())
            .put(
                "nearai",
                ironclaw_secrets::SecretMaterial::from("sk-reborn-stored-nearai-key"),
            )
            .await
            .expect("stored key seeded");
        drop(services);

        let config = ironclaw_llm::LlmConfig {
            backend: "nearai".to_string(),
            session: ironclaw_llm::SessionConfig {
                auth_base_url: base_url.clone(),
                session_path: session_dir.path().join("session.json"),
            },
            nearai: ironclaw_llm::NearAiConfig {
                model: "test-model".to_string(),
                cheap_model: None,
                base_url,
                api_key: None,
                fallback_model: None,
                max_retries: 0,
                circuit_breaker_threshold: None,
                circuit_breaker_recovery_secs: 30,
                response_cache_enabled: false,
                response_cache_ttl_secs: 3600,
                response_cache_max_entries: 1000,
                failover_cooldown_secs: 300,
                failover_cooldown_threshold: 3,
                smart_routing_cascade: false,
            },
            provider: None,
            bedrock: None,
            gemini_oauth: None,
            openai_codex: None,
            request_timeout_secs: 5,
            cheap_model: None,
            smart_routing_cascade: false,
            max_retries: 0,
            circuit_breaker_threshold: None,
            circuit_breaker_recovery_secs: 30,
            response_cache_enabled: false,
            response_cache_ttl_secs: 3600,
            response_cache_max_entries: 1000,
        };
        let llm = crate::runtime_input::ResolvedRebornLlm::from_llm_config(config);

        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-nearai-stored-key-owner", local_dev_root)
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_resolved_llm(llm)
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-nearai-stored-key-tenant".to_string(),
            agent_id: "runtime-nearai-stored-key-agent".to_string(),
            source_binding_id: "runtime-nearai-stored-key-source".to_string(),
            reply_target_binding_id: "runtime-nearai-stored-key-reply".to_string(),
        });

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let reply = runtime
            .send_user_message(&conversation, "hi")
            .await
            .expect("message sends");

        assert!(reply.is_successful_final_reply(), "reply: {reply:?}");
        let auth_header = tokio::time::timeout(Duration::from_secs(5), auth_rx)
            .await
            .expect("chat request should be captured")
            .expect("auth header should be sent by capture server");
        assert_eq!(auth_header, "Bearer sk-reborn-stored-nearai-key");

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn production_runtime_rejects_enabled_hooks_without_local_runtime() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            libsql::Builder::new_local(dir.path().join("reborn.db"))
                .build()
                .await
                .expect("libsql db"),
        );

        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::libsql(
                crate::RebornCompositionProfile::Production,
                "runtime-production-hooks-owner",
                db,
                dir.path().join("events.db").to_string_lossy(),
                None,
                ironclaw_secrets::SecretMaterial::from("01234567890123456789012345678901"),
            )
            .with_production_trust_policy(Arc::new(
                crate::builtin_first_party_trust_policy().expect("trust policy"),
            ))
            .with_runtime_policy(EffectiveRuntimePolicy {
                deployment: DeploymentMode::HostedMultiTenant,
                requested_profile: RuntimeProfile::SecureDefault,
                resolved_profile: RuntimeProfile::SecureDefault,
                filesystem_backend: FilesystemBackendKind::ScopedVirtual,
                process_backend: ProcessBackendKind::TenantSandbox,
                network_mode: NetworkMode::Deny,
                secret_mode: SecretMode::BrokeredHandles,
                approval_policy: ApprovalPolicy::AskAlways,
                audit_mode: AuditMode::Standard,
            })
            .with_runtime_process_binding(RebornRuntimeProcessBinding::tenant_sandbox(Arc::new(
                ironclaw_host_runtime::TenantSandboxProcessPort::new(Arc::new(
                    RecordingSandboxTransport,
                )),
            ))),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-production-hooks-tenant".to_string(),
            agent_id: "runtime-production-hooks-agent".to_string(),
            source_binding_id: "runtime-production-hooks-source".to_string(),
            reply_target_binding_id: "runtime-production-hooks-reply".to_string(),
        })
        .with_hooks_config(HooksActivationConfig::enabled());

        let err = match build_reborn_runtime(input).await {
            Ok(runtime) => {
                runtime.shutdown().await.expect("shutdown");
                panic!("production runtime must reject enabled hooks without hook wiring");
            }
            Err(err) => err,
        };

        assert!(
            matches!(
                err,
                super::RebornRuntimeError::MalformedConfig { ref reason }
                    if reason.contains("hook framework")
                        && reason.contains("production runtime launch")
            ),
            "expected malformed hook config error, got {err:#}"
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn build_reborn_runtime_allows_validated_production_readiness() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            libsql::Builder::new_local(dir.path().join("reborn.db"))
                .build()
                .await
                .expect("libsql db"),
        );
        let gateway = Arc::new(RecordingGateway {
            reply: "validated production runtime".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });

        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::libsql(
                crate::RebornCompositionProfile::Production,
                "runtime-production-cutover-owner",
                db,
                dir.path().join("events.db").to_string_lossy(),
                None,
                ironclaw_secrets::SecretMaterial::from("01234567890123456789012345678901"),
            )
            .with_production_trust_policy(Arc::new(
                crate::builtin_first_party_trust_policy().expect("trust policy"),
            ))
            .with_runtime_policy(EffectiveRuntimePolicy {
                deployment: DeploymentMode::HostedMultiTenant,
                requested_profile: RuntimeProfile::SecureDefault,
                resolved_profile: RuntimeProfile::SecureDefault,
                filesystem_backend: FilesystemBackendKind::ScopedVirtual,
                process_backend: ProcessBackendKind::TenantSandbox,
                network_mode: NetworkMode::Deny,
                secret_mode: SecretMode::BrokeredHandles,
                approval_policy: ApprovalPolicy::AskAlways,
                audit_mode: AuditMode::Standard,
            })
            .with_runtime_process_binding(RebornRuntimeProcessBinding::tenant_sandbox(Arc::new(
                ironclaw_host_runtime::TenantSandboxProcessPort::new(Arc::new(
                    RecordingSandboxTransport,
                )),
            ))),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-production-cutover-tenant".to_string(),
            agent_id: "runtime-production-cutover-agent".to_string(),
            source_binding_id: "runtime-production-cutover-source".to_string(),
            reply_target_binding_id: "runtime-production-cutover-reply".to_string(),
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input)
            .await
            .expect("validated production readiness should start runtime");

        assert_eq!(
            runtime.services().readiness.state,
            RebornReadinessState::ProductionValidated
        );
        assert!(runtime.services().readiness.diagnostics.is_empty());
        assert!(runtime.services().readiness.workers.turn_runner);

        runtime.shutdown().await.expect("runtime shutdown");
    }

    /// Regression guard for Firat's review: a trajectory observer is only wired
    /// through the local-dev capability path, so supplying one to a production
    /// runtime (no local runtime to observe) must fail fast rather than silently
    /// produce an empty trajectory.
    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn build_reborn_runtime_rejects_trajectory_observer_for_production() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            libsql::Builder::new_local(dir.path().join("reborn.db"))
                .build()
                .await
                .expect("libsql db"),
        );
        let gateway = Arc::new(RecordingGateway {
            reply: "production".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });
        let observer = Arc::new(RecordingTrajectoryObserver::default());

        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::libsql(
                crate::RebornCompositionProfile::Production,
                "runtime-observer-reject-owner",
                db,
                dir.path().join("events.db").to_string_lossy(),
                None,
                ironclaw_secrets::SecretMaterial::from("01234567890123456789012345678901"),
            )
            .with_production_trust_policy(Arc::new(
                crate::builtin_first_party_trust_policy().expect("trust policy"),
            ))
            .with_runtime_policy(EffectiveRuntimePolicy {
                deployment: DeploymentMode::HostedMultiTenant,
                requested_profile: RuntimeProfile::SecureDefault,
                resolved_profile: RuntimeProfile::SecureDefault,
                filesystem_backend: FilesystemBackendKind::ScopedVirtual,
                process_backend: ProcessBackendKind::TenantSandbox,
                network_mode: NetworkMode::Deny,
                secret_mode: SecretMode::BrokeredHandles,
                approval_policy: ApprovalPolicy::AskAlways,
                audit_mode: AuditMode::Standard,
            })
            .with_runtime_process_binding(RebornRuntimeProcessBinding::tenant_sandbox(Arc::new(
                ironclaw_host_runtime::TenantSandboxProcessPort::new(Arc::new(
                    RecordingSandboxTransport,
                )),
            ))),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-observer-reject-tenant".to_string(),
            agent_id: "runtime-observer-reject-agent".to_string(),
            source_binding_id: "runtime-observer-reject-source".to_string(),
            reply_target_binding_id: "runtime-observer-reject-reply".to_string(),
        })
        .with_raw_trajectory_observer(observer)
        .with_model_gateway_override(gateway);

        let err = match build_reborn_runtime(input).await {
            Ok(runtime) => {
                runtime.shutdown().await.expect("shutdown");
                panic!("production runtime must reject a trajectory observer");
            }
            Err(err) => err,
        };
        assert!(
            matches!(err, super::RebornRuntimeError::InvalidArgument { ref reason }
                if reason.contains("trajectory observer") && reason.contains("local-dev")),
            "expected an InvalidArgument naming the local-dev-only constraint, got {err:#}"
        );
    }

    #[cfg(feature = "libsql")]
    #[derive(Debug)]
    struct RecordingSandboxTransport;

    #[cfg(feature = "libsql")]
    #[async_trait]
    impl ironclaw_host_runtime::SandboxCommandTransport for RecordingSandboxTransport {
        async fn run_command(
            &self,
            _request: ironclaw_host_runtime::CommandExecutionRequest,
        ) -> Result<
            ironclaw_host_runtime::CommandExecutionOutput,
            ironclaw_host_runtime::RuntimeProcessError,
        > {
            Ok(ironclaw_host_runtime::CommandExecutionOutput {
                output: String::new(),
                saved_output: None,
                exit_code: 0,
                sandboxed: true,
                duration: Duration::ZERO,
            })
        }
    }

    #[tokio::test]
    async fn local_dev_yolo_records_trusted_laptop_access_audit_event() {
        let root = tempfile::tempdir().expect("tempdir");
        let host_home = root.path().join("host-home");
        std::fs::create_dir_all(&host_home).expect("host home");
        let mut policy = local_dev_runtime_policy();
        policy.requested_profile = RuntimeProfile::LocalYolo;
        policy.resolved_profile = RuntimeProfile::LocalYolo;
        policy.filesystem_backend = FilesystemBackendKind::HostWorkspaceAndHome;
        policy.network_mode = NetworkMode::Direct;
        policy.secret_mode = SecretMode::InheritedEnv;

        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev_with_profile(
                crate::RebornCompositionProfile::LocalDevYolo,
                "runtime-yolo-audit-owner",
                root.path().join("local-dev"),
            )
            .with_runtime_policy(policy)
            .with_local_dev_confirmed_host_home_root(host_home),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-yolo-audit-tenant".to_string(),
            agent_id: "runtime-yolo-audit-agent".to_string(),
            source_binding_id: "runtime-yolo-audit-source".to_string(),
            reply_target_binding_id: "runtime-yolo-audit-reply".to_string(),
        });

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let stream = EventStreamKey::new(
            runtime.thread_scope.tenant_id.clone(),
            runtime.actor_user_id.clone(),
            Some(runtime.thread_scope.agent_id.clone()),
        );
        let replay = runtime
            .services
            .local_runtime
            .as_ref()
            .expect("local runtime")
            .audit_log
            .read_after_cursor(&stream, &ReadScope::any(), None, 10)
            .await
            .expect("audit replay");

        let audit = replay
            .entries
            .iter()
            .map(|entry| &entry.record)
            .find(|record| record.action.kind == TRUSTED_LAPTOP_ACCESS_AUDIT_KIND)
            .expect("trusted laptop access audit event");
        assert_eq!(audit.stage, AuditStage::After);
        assert_eq!(
            audit.action.target.as_deref(),
            Some(TRUSTED_LAPTOP_ACCESS_AUDIT_TARGET)
        );
        assert_eq!(
            audit
                .result
                .as_ref()
                .and_then(|result| result.status.as_deref()),
            Some(TRUSTED_LAPTOP_ACCESS_AUDIT_STATUS)
        );
        assert_eq!(audit.decision.kind, "allowed");
        runtime.shutdown().await.expect("shutdown");
    }

    #[tokio::test]
    async fn local_dev_runtime_readiness_reports_trigger_poller_worker() {
        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(RecordingGateway {
            reply: "trigger readiness".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });

        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(
                "runtime-trigger-readiness-owner",
                root.path().join("local-dev"),
            )
            .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-trigger-readiness-tenant".to_string(),
            agent_id: "runtime-trigger-readiness-agent".to_string(),
            source_binding_id: "runtime-trigger-readiness-source".to_string(),
            reply_target_binding_id: "runtime-trigger-readiness-reply".to_string(),
        })
        .with_trigger_poller_settings(
            TriggerPollerSettings::enabled_with_tenant_scoped_authorizer_for_test(),
        )
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");

        assert!(runtime.services().readiness.workers.turn_runner);
        assert!(runtime.services().readiness.workers.trigger_poller);

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_runtime_rejects_trigger_poller_without_creator_authorization() {
        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(RecordingGateway {
            reply: "trigger auth required".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });

        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(
                "runtime-trigger-auth-required-owner",
                root.path().join("local-dev"),
            )
            .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-trigger-auth-required-tenant".to_string(),
            agent_id: "runtime-trigger-auth-required-agent".to_string(),
            source_binding_id: "runtime-trigger-auth-required-source".to_string(),
            reply_target_binding_id: "runtime-trigger-auth-required-reply".to_string(),
        })
        .with_trigger_poller_settings(TriggerPollerSettings::enabled())
        .with_model_gateway_override(gateway);

        let err = match build_reborn_runtime(input).await {
            Ok(runtime) => {
                runtime
                    .shutdown()
                    .await
                    .expect("unexpected runtime shutdown");
                panic!(
                    "creator-access-required setting must not enable trigger poller without an access checker"
                );
            }
            Err(err) => err,
        };

        assert!(
            matches!(err, super::RebornRuntimeError::InvalidArgument { reason } if reason.contains("fire-time creator access checker"))
        );
    }

    #[tokio::test]
    async fn local_dev_runtime_accepts_trigger_poller_with_creator_access_checker() {
        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(RecordingGateway {
            reply: "trigger auth supplied".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });

        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(
                "runtime-trigger-auth-supplied-owner",
                root.path().join("local-dev"),
            )
            .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-trigger-auth-supplied-tenant".to_string(),
            agent_id: "runtime-trigger-auth-supplied-agent".to_string(),
            source_binding_id: "runtime-trigger-auth-supplied-source".to_string(),
            reply_target_binding_id: "runtime-trigger-auth-supplied-reply".to_string(),
        })
        .with_trigger_poller_settings(TriggerPollerSettings::enabled())
        .with_trigger_fire_access_checker(Arc::new(AllowingTriggerFireAccessChecker))
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input)
            .await
            .expect("runtime builds with creator access checker");

        assert!(runtime.services().readiness.workers.turn_runner);
        assert!(runtime.services().readiness.workers.trigger_poller);

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_runtime_disables_trigger_poller_worker_by_default() {
        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(RecordingGateway {
            reply: "trigger disabled".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });

        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(
                "runtime-trigger-disabled-owner",
                root.path().join("local-dev"),
            )
            .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-trigger-disabled-tenant".to_string(),
            agent_id: "runtime-trigger-disabled-agent".to_string(),
            source_binding_id: "runtime-trigger-disabled-source".to_string(),
            reply_target_binding_id: "runtime-trigger-disabled-reply".to_string(),
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");

        assert!(runtime.services().readiness.workers.turn_runner);
        assert!(!runtime.services().readiness.workers.trigger_poller);

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_runtime_rejects_invalid_trigger_poller_worker_config() {
        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(RecordingGateway {
            reply: "trigger invalid config".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });
        let trigger_poller = TriggerPollerSettings {
            enabled: true,
            worker: ironclaw_triggers::TriggerPollerWorkerConfig {
                poll_interval: Duration::ZERO,
                ..Default::default()
            },
            ..Default::default()
        }
        .with_tenant_scoped_authorizer_for_test();

        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(
                "runtime-trigger-invalid-config-owner",
                root.path().join("local-dev"),
            )
            .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-trigger-invalid-config-tenant".to_string(),
            agent_id: "runtime-trigger-invalid-config-agent".to_string(),
            source_binding_id: "runtime-trigger-invalid-config-source".to_string(),
            reply_target_binding_id: "runtime-trigger-invalid-config-reply".to_string(),
        })
        .with_trigger_poller_settings(trigger_poller)
        .with_model_gateway_override(gateway);

        let err = match build_reborn_runtime(input).await {
            Ok(runtime) => {
                runtime
                    .shutdown()
                    .await
                    .expect("unexpected runtime shutdown");
                panic!("invalid trigger poller config must fail runtime build");
            }
            Err(err) => err,
        };

        assert!(
            matches!(err, super::RebornRuntimeError::InvalidArgument { reason } if reason.contains("poll_interval must be non-zero"))
        );
    }

    #[tokio::test]
    async fn local_dev_runtime_shutdown_cancels_trigger_poller_worker() {
        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(RecordingGateway {
            reply: "trigger shutdown".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });

        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(
                "runtime-trigger-shutdown-owner",
                root.path().join("local-dev"),
            )
            .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-trigger-shutdown-tenant".to_string(),
            agent_id: "runtime-trigger-shutdown-agent".to_string(),
            source_binding_id: "runtime-trigger-shutdown-source".to_string(),
            reply_target_binding_id: "runtime-trigger-shutdown-reply".to_string(),
        })
        .with_trigger_poller_settings(
            TriggerPollerSettings::enabled_with_tenant_scoped_authorizer_for_test(),
        )
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        assert!(runtime.services().readiness.workers.trigger_poller);

        tokio::time::timeout(std::time::Duration::from_secs(2), runtime.shutdown())
            .await
            .expect("shutdown returns before timeout")
            .expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_yolo_message_flow_ignores_model_budget_gate() {
        let root = tempfile::tempdir().expect("tempdir");
        let host_home = root.path().join("host-home");
        std::fs::create_dir_all(&host_home).expect("host home");
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let gateway = Arc::new(RecordingGateway {
            reply: "yolo budget bypass reply".to_string(),
            requests: Arc::clone(&requests),
        });
        let cost_table = ironclaw_loop_support::StaticModelCostTable::new().with_entry(
            ModelProfileId::new("interactive_model").expect("model profile id"),
            ModelCost {
                input_per_token: dec!(1.00),
                output_per_token: dec!(1.00),
                max_output_tokens: 8_192,
            },
        );

        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev_with_profile(
                crate::RebornCompositionProfile::LocalDevYolo,
                "runtime-yolo-budget-owner",
                root.path().join("local-dev"),
            )
            .with_runtime_policy(
                crate::local_dev_yolo_runtime_policy(true).expect("local-yolo policy resolves"),
            )
            .with_local_dev_confirmed_host_home_root(host_home),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-yolo-budget-tenant".to_string(),
            agent_id: "runtime-yolo-budget-agent".to_string(),
            source_binding_id: "runtime-yolo-budget-source".to_string(),
            reply_target_binding_id: "runtime-yolo-budget-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_model_gateway_override(gateway)
        .with_model_cost_table_override(Arc::new(cost_table));

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let reply = tokio::time::timeout(
            RUNTIME_SEND_TIMEOUT,
            runtime.send_user_message(&conversation, "ping"),
        )
        .await
        .expect("runtime send should finish")
        .expect("runtime send should succeed");

        assert_eq!(reply.status, TurnStatus::Completed);
        assert_eq!(reply.text.as_deref(), Some("yolo budget bypass reply"));
        assert_eq!(
            recorded_request_count(&requests),
            1,
            "local-dev-yolo must reach the model gateway even when a paid cost table is present"
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn send_user_message_returns_completed_assistant_text_with_recording_gateway() {
        let root = tempfile::tempdir().expect("tempdir");
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let gateway = Arc::new(RecordingGateway {
            reply: "recorded runtime reply".to_string(),
            requests: Arc::clone(&requests),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-success-owner", root.path().join("local-dev"))
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-success-tenant".to_string(),
            agent_id: "runtime-success-agent".to_string(),
            source_binding_id: "runtime-success-source".to_string(),
            reply_target_binding_id: "runtime-success-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: RUNTIME_POLL_TIMEOUT,
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let local_runtime = runtime
            .services
            .local_runtime
            .as_ref()
            .expect("runtime should use local-dev RebornServices substrate");
        assert!(
            Arc::ptr_eq(&runtime.thread_service, &local_runtime.thread_service),
            "REPL runtime should use the thread service owned by RebornServices"
        );
        assert!(
            Arc::ptr_eq(
                &runtime.turn_coordinator,
                runtime
                    .services
                    .turn_coordinator
                    .as_ref()
                    .expect("RebornServices turn coordinator")
            ),
            "REPL runtime should drive turns through RebornServices"
        );
        let conversation = runtime.new_conversation().await.expect("conversation");
        let reply = tokio::time::timeout(
            RUNTIME_SEND_TIMEOUT,
            runtime.send_user_message(&conversation, "ping"),
        )
        .await
        .expect("runtime send should finish")
        .expect("runtime send should succeed");

        assert_eq!(reply.status, TurnStatus::Completed);
        assert_eq!(reply.text.as_deref(), Some("recorded runtime reply"));
        assert_eq!(recorded_request_count(&requests), 1);

        runtime.shutdown().await.expect("runtime shutdown");
    }

    /// End-to-end Trace Commons auto-capture: a real runtime turn through
    /// `send_user_message` must, for an enrolled owner scope, land a redacted
    /// envelope in that scope's submission queue without any manual trace
    /// command. This drives the full chain: turn completion → lifecycle bus →
    /// best-effort capture sink → thread-history read → redact/score →
    /// eligibility → queue (+ immediate flush attempt, which fails locally
    /// against the closed loopback endpoint and must leave the entry queued).
    #[tokio::test]
    async fn send_user_message_auto_queues_trace_for_enrolled_scope() {
        use ironclaw_reborn_traces::contribution as trace_contribution;

        let owner = format!("runtime-trace-capture-owner-{}", uuid::Uuid::new_v4());
        // Trace state is keyed by the tenant-scoped composite, so enroll (and
        // later read the queue) under `trace_scope_key(tenant, owner)`, not the
        // bare owner id.
        let scope = trace_contribution::trace_scope_key("runtime-trace-capture-tenant", &owner);
        let policy = trace_contribution::StandingTraceContributionPolicy {
            enabled: true,
            // Closed loopback port: the immediate flush fails fast and
            // locally; no traffic leaves the machine.
            ingestion_endpoint: Some("https://127.0.0.1:1/v1/traces".to_string()),
            min_submission_score: 0.0,
            require_manual_approval_when_pii_detected: false,
            auto_submit_high_value_traces: true,
            ..trace_contribution::StandingTraceContributionPolicy::default()
        };
        trace_contribution::write_trace_policy_for_scope(Some(&scope), &policy)
            .expect("write trace policy");

        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(RecordingGateway {
            reply: "auto capture reply".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(&owner, root.path().join("local-dev"))
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-trace-capture-tenant".to_string(),
            agent_id: "runtime-trace-capture-agent".to_string(),
            source_binding_id: "runtime-trace-capture-source".to_string(),
            reply_target_binding_id: "runtime-trace-capture-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: RUNTIME_POLL_TIMEOUT,
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let reply = tokio::time::timeout(
            RUNTIME_SEND_TIMEOUT,
            runtime.send_user_message(&conversation, "capture this turn"),
        )
        .await
        .expect("runtime send should finish")
        .expect("runtime send should succeed");
        assert_eq!(reply.status, TurnStatus::Completed);

        // The capture task is detached from the lifecycle path; poll briefly.
        let queue_dir =
            trace_contribution::trace_contribution_dir_for_scope(Some(&scope)).join("queue");
        let queued = |dir: &std::path::Path| -> Vec<std::path::PathBuf> {
            match std::fs::read_dir(dir) {
                Ok(entries) => entries
                    .map(|entry| {
                        // Fail loud on a per-entry IO error too, so the test
                        // can't silently drop a broken entry and still claim the
                        // queue holds exactly one envelope.
                        entry
                            .unwrap_or_else(|error| {
                                panic!(
                                    "failed to read a trace queue entry in {}: {error}",
                                    dir.display()
                                )
                            })
                            .path()
                    })
                    .filter(|path| {
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| {
                                name.ends_with(".json") && !name.ends_with(".held.json")
                            })
                    })
                    .collect(),
                // The queue dir not existing yet is the expected pre-capture
                // state; any other IO error is a real failure the test must not
                // mask as "no queued traces".
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                Err(error) => panic!("failed to read trace queue dir {}: {error}", dir.display()),
            }
        };
        let mut entries = Vec::new();
        for _ in 0..150 {
            entries = queued(&queue_dir);
            if !entries.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(
            entries.len(),
            1,
            "a completed turn for an enrolled scope must auto-queue one trace envelope"
        );
        let body = std::fs::read_to_string(&entries[0]).expect("queued envelope readable");
        let envelope: serde_json::Value = serde_json::from_str(&body).expect("envelope is JSON");
        assert_eq!(envelope["outcome"]["task_success"], "success");

        runtime.shutdown().await.expect("runtime shutdown");
        let _ = std::fs::remove_dir_all(trace_contribution::trace_contribution_dir_for_scope(
            Some(&scope),
        ));
    }

    /// Regression guard: `send_user_message` must persist a
    /// `TurnOwner::Personal` (the bound actor user) in `product_context`,
    /// not a `TurnOwner::SharedAgent`.  Before the fix, `turn_scope_for`
    /// built an ownerless scope whose `product_owner` resolved to
    /// `SharedAgent` because `agent_id` was set and no explicit owner was
    /// carried.
    #[tokio::test(flavor = "multi_thread")]
    async fn send_user_message_persists_personal_owner_for_webui() {
        use ironclaw_turns::TurnOwner;

        let root = tempfile::tempdir().expect("tempdir");
        let actor_owner_id = "runtime-personal-owner-user";
        let gateway = Arc::new(RecordingGateway {
            reply: "owner-check reply".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(actor_owner_id, root.path().join("local-dev"))
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-personal-owner-tenant".to_string(),
            agent_id: "runtime-personal-owner-agent".to_string(),
            source_binding_id: "runtime-personal-owner-source".to_string(),
            reply_target_binding_id: "runtime-personal-owner-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: RUNTIME_POLL_TIMEOUT,
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let reply = tokio::time::timeout(
            RUNTIME_SEND_TIMEOUT,
            runtime.send_user_message(&conversation, "ping"),
        )
        .await
        .expect("runtime send should finish within timeout")
        .expect("runtime send should succeed");

        assert_eq!(reply.status, TurnStatus::Completed);

        // Verify the persisted product_context carries Personal{user: actor_user_id},
        // not SharedAgent.
        let scope = runtime.turn_scope_for(&conversation.0);
        let run_state = runtime
            .turn_coordinator
            .get_run_state(GetRunStateRequest {
                scope,
                run_id: reply.run_id,
            })
            .await
            .expect("get_run_state should succeed");

        let product_context = run_state
            .product_context
            .expect("product_context must be set by send_user_message");
        let expected_user_id = UserId::new(actor_owner_id).expect("actor user id should be valid");
        assert!(
            matches!(
                &product_context.owner,
                TurnOwner::Personal { user } if user == &expected_user_id
            ),
            "send_user_message must persist TurnOwner::Personal{{user: actor_user_id}}, \
             got {:?}",
            product_context.owner
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    /// Regression guard: `send_user_message` resolves product context via
    /// `resolve_web_ui`, which sets `TurnOriginKind::WebUi`.  The runtime
    /// context section rendered into the model request must therefore contain
    /// the WebUI origin line produced by
    /// `LoopRuntimeContext::render_model_content`.  Previously, only the
    /// persisted `product_context` owner was asserted; this test closes the
    /// gap by asserting the *rendered* origin appears in the captured model
    /// request.
    #[tokio::test]
    async fn send_user_message_renders_webui_origin_in_model_request() {
        let root = tempfile::tempdir().expect("tempdir");
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let gateway = Arc::new(RecordingGateway {
            reply: "webui-origin-check reply".to_string(),
            requests: Arc::clone(&requests),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(
                "runtime-webui-origin-owner",
                root.path().join("local-dev"),
            )
            .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-webui-origin-tenant".to_string(),
            agent_id: "runtime-webui-origin-agent".to_string(),
            source_binding_id: "runtime-webui-origin-source".to_string(),
            reply_target_binding_id: "runtime-webui-origin-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: RUNTIME_POLL_TIMEOUT,
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let reply = tokio::time::timeout(
            RUNTIME_SEND_TIMEOUT,
            runtime.send_user_message(&conversation, "ping"),
        )
        .await
        .expect("runtime send should finish within timeout")
        .expect("runtime send should succeed");

        assert_eq!(reply.status, TurnStatus::Completed);

        // The runtime-context system message carries the rendered
        // `LoopRuntimeContext` — its content_ref uses the "runtime" section
        // prefix stamped by `push_runtime_context`.
        let runtime_context_content = {
            let requests = requests
                .lock()
                .expect("recording gateway requests lock poisoned");
            requests[0]
                .messages
                .iter()
                .find(|message| {
                    message.role == HostManagedModelMessageRole::System
                        && message
                            .content_ref
                            .as_str()
                            .starts_with("msg:runtime.loop-start.")
                })
                .expect(
                    "model request must include a runtime-context system message \
                     (content_ref starts with msg:runtime.loop-start.)",
                )
                .content
                .clone()
        };

        // Exact string produced by LoopRuntimeContext::render_model_content
        // for TurnOriginKind::WebUi (runtime_context.rs line 225).
        assert!(
            runtime_context_content
                .contains("Run origin: WebUI chat; replies render in this chat."),
            "runtime-context system message must contain the WebUI origin line, \
             got: {runtime_context_content:?}"
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn send_user_message_until_gate_returns_blocked_on_auth_gate() {
        let root = tempfile::tempdir().expect("tempdir");
        let host_home = root.path().join("host-home");
        std::fs::create_dir_all(&host_home).expect("host home");
        let gateway = Arc::new(AuthGateToolCallingGateway::default());
        let gateway_for_runtime: Arc<dyn HostManagedModelGateway> = gateway.clone();
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev_with_profile(
                RebornCompositionProfile::LocalDevYolo,
                "runtime-auth-gate-owner",
                root.path().join("local-dev"),
            )
            .with_runtime_policy(
                crate::local_dev_yolo_runtime_policy(true).expect("local-yolo policy resolves"),
            )
            .with_local_dev_confirmed_host_home_root(host_home),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-auth-gate-tenant".to_string(),
            agent_id: "runtime-auth-gate-agent".to_string(),
            source_binding_id: "runtime-auth-gate-source".to_string(),
            reply_target_binding_id: "runtime-auth-gate-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: RUNTIME_POLL_TIMEOUT,
        })
        .with_model_gateway_override(gateway_for_runtime);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let local_runtime = runtime
            .services
            .local_runtime
            .as_ref()
            .expect("local runtime services");
        let extension_management = local_runtime
            .extension_management
            .as_ref()
            .expect("extension management");
        let notion_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion")
            .expect("valid notion ref");
        extension_management
            .install(notion_ref.clone())
            .await
            .expect("install Notion MCP");
        extension_management
            .activate_with_prechecked_credentials_for_test(
                notion_ref,
                ExtensionActivationMode::Static,
            )
            .await
            .expect("activate Notion MCP");

        let conversation = runtime.new_conversation().await.expect("conversation");
        let outcome = tokio::time::timeout(
            RUNTIME_SEND_TIMEOUT,
            runtime.send_user_message_until_gate(&conversation, "search Notion"),
        )
        .await
        .expect("gate-aware send should return before timeout")
        .expect("gate-aware send should succeed");

        let (run_id, gate_ref) = match outcome {
            super::RebornTurnDriveOutcome::BlockedOnGate {
                run_id,
                status,
                gate_ref,
                ..
            } => {
                assert_eq!(status, TurnStatus::BlockedAuth);
                assert!(
                    gate_ref.as_str().starts_with("gate:auth-"),
                    "auth gate ref should carry the auth prefix, got {}",
                    gate_ref.as_str()
                );
                (run_id, gate_ref)
            }
            super::RebornTurnDriveOutcome::Terminal(reply) => {
                panic!("auth-gated turn should pause before terminal reply, got {reply:?}");
            }
        };
        let state = runtime
            .turn_coordinator
            .get_run_state(GetRunStateRequest {
                scope: runtime.turn_scope_for(&conversation.0),
                run_id,
            })
            .await
            .expect("blocked run state");
        assert_eq!(state.status, TurnStatus::BlockedAuth);
        assert_eq!(state.gate_ref.as_ref(), Some(&gate_ref));

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn cancel_run_propagates_to_subagent_children() {
        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(RecordingGateway {
            reply: "unused".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(
                "runtime-cancel-child-owner",
                root.path().join("local-dev"),
            )
            .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-cancel-child-tenant".to_string(),
            agent_id: "runtime-cancel-child-agent".to_string(),
            source_binding_id: "runtime-cancel-child-source".to_string(),
            reply_target_binding_id: "runtime-cancel-child-reply".to_string(),
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        stop_turn_runner_worker_for_manual_state_test(&runtime).await;
        let conversation = runtime.new_conversation().await.expect("conversation");
        let parent_scope = runtime.turn_scope_for(&conversation.0);
        let actor = TurnActor::new(runtime.actor_user_id.clone());
        let parent = runtime
            .turn_coordinator
            .submit_turn(SubmitTurnRequest {
                scope: parent_scope.clone(),
                actor: actor.clone(),
                accepted_message_ref: AcceptedMessageRef::new("msg:cancel-parent").unwrap(),
                source_binding_ref: SourceBindingRef::new("source:cancel-parent").unwrap(),
                reply_target_binding_ref: ReplyTargetBindingRef::new("reply:cancel-parent")
                    .unwrap(),
                requested_run_profile: None,
                idempotency_key: IdempotencyKey::new("cancel-parent").unwrap(),
                received_at: Utc::now(),
                requested_run_id: None,
                parent_run_id: None,
                subagent_depth: 0,
                spawn_tree_root_run_id: None,
                product_context: None,
            })
            .await
            .expect("parent submitted");
        let SubmitTurnResponse::Accepted {
            run_id: parent_run_id,
            ..
        } = parent;
        let child_scope = TurnScope::new(
            parent_scope.tenant_id.clone(),
            parent_scope.agent_id.clone(),
            parent_scope.project_id.clone(),
            ThreadId::new("runtime-cancel-child-thread").unwrap(),
        );
        let child = runtime
            .turn_tree_store
            .submit_child_turn(
                SubmitChildRunRequest {
                    parent_scope: parent_scope.clone(),
                    parent_run_id,
                    child_scope: child_scope.clone(),
                    actor,
                    accepted_message_ref: AcceptedMessageRef::new("msg:cancel-child").unwrap(),
                    source_binding_ref: SourceBindingRef::new("source:cancel-child").unwrap(),
                    reply_target_binding_ref: ReplyTargetBindingRef::new("reply:cancel-child")
                        .unwrap(),
                    requested_run_profile: None,
                    idempotency_key: IdempotencyKey::new("cancel-child").unwrap(),
                    received_at: Utc::now(),
                    requested_run_id: None,
                    spawn_tree_descendant_cap: 4,
                },
                &AllowAllTurnAdmissionPolicy,
                &InMemoryRunProfileResolver::default(),
            )
            .await
            .expect("child submitted");
        let SubmitTurnResponse::Accepted {
            run_id: child_run_id,
            ..
        } = child;

        runtime
            .cancel_run(
                &parent_scope,
                parent_run_id,
                SanitizedCancelReason::UserRequested,
                "test-parent-cancel",
            )
            .await
            .expect("parent cancellation succeeds");

        let result_ref = LoopResultRef::new("result:runtime-cancel-child").unwrap();
        runtime
            .thread_service
            .append_tool_result_reference(AppendToolResultReferenceRequest {
                scope: runtime.thread_scope.clone(),
                thread_id: parent_scope.thread_id.clone(),
                turn_run_id: parent_run_id.to_string(),
                result_ref: result_ref.as_str().to_string(),
                safe_summary: ToolResultSafeSummary::new("subagent spawned").unwrap(),
                provider_call: None,
                model_observation: None,
            })
            .await
            .expect("parent result reference seeded");
        let child_thread_scope = ThreadScope {
            tenant_id: child_scope.tenant_id.clone(),
            agent_id: child_scope.agent_id.clone().unwrap(),
            project_id: child_scope.project_id.clone(),
            owner_user_id: Some(runtime.actor_user_id.clone()),
            mission_id: None,
        };
        runtime
            .thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: child_thread_scope,
                thread_id: Some(child_scope.thread_id.clone()),
                created_by_actor_id: "test".to_string(),
                title: Some("Subagent".to_string()),
                metadata_json: Some(
                    serde_json::to_string(&SubagentThreadMetadata {
                        kind: SubagentThreadKind::Subagent,
                        parent_run_id,
                        parent_thread_id: parent_scope.thread_id.clone(),
                        tree_root_run_id: parent_run_id,
                        child_run_id,
                        subagent_kind: SubagentKindId::new("general").unwrap(),
                        mode: SpawnSubagentMode::Blocking,
                        result_ref,
                        handoff: None,
                    })
                    .unwrap(),
                ),
            })
            .await
            .expect("child thread metadata seeded");

        let child_state = runtime
            .turn_coordinator
            .get_run_state(GetRunStateRequest {
                scope: child_scope,
                run_id: child_run_id,
            })
            .await
            .expect("child state");
        assert_eq!(child_state.status, TurnStatus::Cancelled);

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn send_user_message_uses_caller_supplied_skill_context_source() {
        let root = tempfile::tempdir().expect("tempdir");
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let gateway = Arc::new(RecordingGateway {
            reply: "should not reach model".to_string(),
            requests: Arc::clone(&requests),
        });
        let skill_context_source = Arc::new(FailingSkillContextSource::default());
        let skill_context_source_for_input: Arc<dyn HostSkillContextSource> =
            skill_context_source.clone();
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-skill-owner", root.path().join("local-dev"))
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-skill-tenant".to_string(),
            agent_id: "runtime-skill-agent".to_string(),
            source_binding_id: "runtime-skill-source".to_string(),
            reply_target_binding_id: "runtime-skill-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_skill_context_source(skill_context_source_for_input)
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let reply = tokio::time::timeout(
            RUNTIME_SEND_TIMEOUT,
            runtime.send_user_message(&conversation, "ping"),
        )
        .await
        .expect("runtime send should finish")
        .expect("runtime send should succeed");

        assert_ne!(reply.status, TurnStatus::Completed);
        assert_eq!(
            skill_context_source.calls.load(Ordering::SeqCst),
            1,
            "composition should pass caller-supplied skill context into the planned runtime"
        );
        assert!(
            requests
                .lock()
                .expect("recording gateway requests lock poisoned")
                .is_empty(),
            "skill context failure should stop before model dispatch"
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_runtime_exposes_host_runtime_capabilities_to_model_calls() {
        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(ToolCallingGateway::default());
        let gateway_for_runtime: Arc<dyn HostManagedModelGateway> = gateway.clone();
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-tools-owner", root.path().join("local-dev"))
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-tools-tenant".to_string(),
            agent_id: "runtime-tools-agent".to_string(),
            source_binding_id: "runtime-tools-source".to_string(),
            reply_target_binding_id: "runtime-tools-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_model_gateway_override(gateway_for_runtime);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let reply = tokio::time::timeout(
            RUNTIME_SEND_TIMEOUT,
            runtime.send_user_message(&conversation, "use echo tool"),
        )
        .await
        .expect("runtime send should finish")
        .expect("runtime send should succeed");

        assert_eq!(reply.status, TurnStatus::Completed);
        assert_eq!(reply.text.as_deref(), Some("tool ok"));
        assert_eq!(
            *gateway
                .stream_model_calls
                .lock()
                .expect("tool gateway stream count lock poisoned"),
            0,
            "runtime should use capability-aware model path"
        );
        assert_eq!(
            gateway
                .requests
                .lock()
                .expect("tool gateway requests lock poisoned")
                .len(),
            2,
            "tool call should require initial request plus tool-result follow-up"
        );
        let history = runtime
            .thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: runtime.thread_scope.clone(),
                thread_id: conversation.0.clone(),
            })
            .await
            .expect("thread history");
        let tool_result = history
            .messages
            .iter()
            .find(|message| message.kind == MessageKind::ToolResultReference)
            .expect("tool result reference should persist in thread history");
        assert!(
            tool_result
                .tool_result_ref
                .as_deref()
                .is_some_and(|result_ref| result_ref.starts_with("result:")),
            "tool result should persist a durable result ref"
        );
        assert!(
            tool_result.tool_result_provider_call.is_none(),
            "product thread history should scrub provider replay metadata"
        );
        let context = runtime
            .thread_service
            .load_context_messages(LoadContextMessagesRequest {
                scope: runtime.thread_scope.clone(),
                thread_id: conversation.0.clone(),
                message_ids: vec![tool_result.message_id],
            })
            .await
            .expect("tool result context");
        let provider_call = context.messages[0]
            .tool_result_provider_call
            .as_ref()
            .expect("model context should preserve provider replay metadata");
        assert_eq!(provider_call.provider_call_id, "call-1");
        assert_eq!(
            provider_call.capability_id,
            CapabilityId::new("builtin.echo").unwrap()
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    /// Records both trajectory callbacks so the e2e test can assert the
    /// observer fires through a real `build_reborn_runtime` turn — driving the
    /// input hook (`HostRuntimeLoopCapabilityPort`) and the result hook
    /// (`LocalDevCapabilityIo::write_capability_result`) on the actual dispatch
    /// path, not a direct helper call.
    #[derive(Debug, Default)]
    struct RecordingTrajectoryObserver {
        inputs: StdMutex<Vec<(String, String, serde_json::Value)>>,
        results: StdMutex<Vec<(String, String, serde_json::Value)>>,
    }

    impl crate::RebornTrajectoryObserver for RecordingTrajectoryObserver {
        fn on_capability_input(
            &self,
            call_id: &str,
            capability_id: &str,
            arguments: &serde_json::Value,
        ) {
            self.inputs.lock().expect("inputs lock").push((
                call_id.to_string(),
                capability_id.to_string(),
                arguments.clone(),
            ));
        }

        fn on_capability_result(
            &self,
            call_id: &str,
            capability_id: &str,
            output: &serde_json::Value,
        ) {
            self.results.lock().expect("results lock").push((
                call_id.to_string(),
                capability_id.to_string(),
                output.clone(),
            ));
        }
    }

    /// End-to-end guard for the #4588 trajectory observer seam: a real runtime
    /// turn that dispatches the `builtin.echo` capability must fire BOTH the
    /// input and result callbacks installed via
    /// `RebornRuntimeInput::with_raw_trajectory_observer`. This drives the
    /// result hook on the genuine dispatch path (the prior direct-call unit
    /// test was dropped as false confidence — it stayed green even when
    /// end-to-end dispatch was broken).
    #[tokio::test]
    async fn local_dev_runtime_forwards_tool_call_trajectory_to_raw_observer() {
        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(ToolCallingGateway::default());
        let gateway_for_runtime: Arc<dyn HostManagedModelGateway> = gateway.clone();
        let observer = Arc::new(RecordingTrajectoryObserver::default());
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-trajectory-owner", root.path().join("local-dev"))
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-trajectory-tenant".to_string(),
            agent_id: "runtime-trajectory-agent".to_string(),
            source_binding_id: "runtime-trajectory-source".to_string(),
            reply_target_binding_id: "runtime-trajectory-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        // Raw (not safe-preview) so we can assert verbatim arguments + output.
        .with_raw_trajectory_observer(observer.clone())
        .with_model_gateway_override(gateway_for_runtime);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let reply = tokio::time::timeout(
            RUNTIME_SEND_TIMEOUT,
            runtime.send_user_message(&conversation, "use echo tool"),
        )
        .await
        .expect("runtime send should finish")
        .expect("runtime send should succeed");
        assert_eq!(reply.status, TurnStatus::Completed);
        // Shut down before inspecting the recorded callbacks so the std-Mutex
        // guards are never held across an `.await` (clippy::await_holding_lock).
        runtime.shutdown().await.expect("runtime shutdown");

        let echo_id = CapabilityId::new("builtin.echo").unwrap();

        let inputs = observer.inputs.lock().expect("inputs lock");
        assert_eq!(inputs.len(), 1, "exactly one capability input observed");
        let (input_call_id, input_capability, arguments) = &inputs[0];
        assert!(!input_call_id.is_empty(), "input call_id should be present");
        assert_eq!(input_capability, echo_id.as_str());
        assert_eq!(
            arguments,
            &serde_json::json!({"message": "hello from tool"}),
            "observer should receive the raw model-emitted tool arguments"
        );

        let results = observer.results.lock().expect("results lock");
        assert_eq!(results.len(), 1, "exactly one capability result observed");
        let (result_call_id, result_capability, output) = &results[0];
        assert_eq!(result_capability, echo_id.as_str());
        assert_eq!(
            result_call_id, input_call_id,
            "result and input callbacks correlate by call_id"
        );
        assert!(
            output.to_string().contains("hello from tool"),
            "observer should receive the staged capability output, got {output}"
        );
    }

    /// Caller-level guard for the **default** (safe-preview) observer path:
    /// installing via the public `with_trajectory_observer` and driving a real
    /// turn with a large tool payload must deliver a *bounded* preview to the
    /// observer — proving truncation is wired between dispatch and the observer,
    /// not just unit-tested on the helper in isolation.
    #[tokio::test]
    async fn local_dev_runtime_safe_preview_observer_receives_bounded_payload() {
        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(LargeEchoToolCallingGateway::default());
        let gateway_for_runtime: Arc<dyn HostManagedModelGateway> = gateway.clone();
        let observer = Arc::new(RecordingTrajectoryObserver::default());
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-preview-owner", root.path().join("local-dev"))
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-preview-tenant".to_string(),
            agent_id: "runtime-preview-agent".to_string(),
            source_binding_id: "runtime-preview-source".to_string(),
            reply_target_binding_id: "runtime-preview-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        // Default path → safe-preview truncation applied before the observer.
        .with_trajectory_observer(observer.clone())
        .with_model_gateway_override(gateway_for_runtime);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let reply = tokio::time::timeout(
            RUNTIME_SEND_TIMEOUT,
            runtime.send_user_message(&conversation, "echo a big payload"),
        )
        .await
        .expect("runtime send should finish")
        .expect("runtime send should succeed");
        assert_eq!(reply.status, TurnStatus::Completed);
        // Shut down before inspecting the recorded callbacks so the std-Mutex
        // guards are never held across an `.await` (clippy::await_holding_lock).
        runtime.shutdown().await.expect("runtime shutdown");

        let original_len = LARGE_ECHO_MESSAGE.repeat(100).len();

        let inputs = observer.inputs.lock().expect("inputs lock");
        assert_eq!(inputs.len(), 1, "exactly one capability input observed");
        let observed_message = inputs[0].2["message"].as_str().expect("message string");
        assert!(
            observed_message.len() < original_len && observed_message.contains("[truncated"),
            "observer should receive a truncated preview of the large argument, got {} bytes",
            observed_message.len()
        );

        let results = observer.results.lock().expect("results lock");
        assert_eq!(results.len(), 1, "exactly one capability result observed");
        assert!(
            results[0].2.to_string().contains("[truncated"),
            "observer should receive a truncated preview of the large result"
        );
    }

    #[tokio::test]
    async fn local_dev_runtime_wires_input_skill_context_source_to_model_calls() {
        let root = tempfile::tempdir().expect("tempdir");
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let gateway = Arc::new(RecordingGateway {
            reply: "skill context ok".to_string(),
            requests: Arc::clone(&requests),
        });
        let skill_source = Arc::new(StaticSkillContextSource::new(vec![
            HostSkillContextCandidate::loaded(
                skill_md(
                    "review-helper",
                    "review helper description",
                    "Use review helper prompt content.",
                ),
                Some(SkillTrust::Trusted),
                Some(SkillVisibility::Visible),
            ),
        ]));
        let skill_context_source: Arc<dyn HostSkillContextSource> = skill_source;
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-skill-owner", root.path().join("local-dev"))
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-skill-tenant".to_string(),
            agent_id: "runtime-skill-agent".to_string(),
            source_binding_id: "runtime-skill-source".to_string(),
            reply_target_binding_id: "runtime-skill-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_skill_context_source(skill_context_source)
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let reply = tokio::time::timeout(
            RUNTIME_SEND_TIMEOUT,
            runtime.send_user_message(&conversation, "review this"),
        )
        .await
        .expect("runtime send should finish")
        .expect("runtime send should succeed");

        assert_eq!(reply.status, TurnStatus::Completed);
        assert_eq!(reply.text.as_deref(), Some("skill context ok"));
        let (request_count, skill_message_content) = {
            let requests = requests
                .lock()
                .expect("recording gateway requests lock poisoned");
            let skill_message = requests[0]
                .messages
                .iter()
                .find(|message| {
                    message.role == HostManagedModelMessageRole::System
                        && message
                            .content_ref
                            .as_str()
                            .starts_with("msg:snippet.skill.review-helper.")
                })
                .expect("model request should include skill-context system message");
            (requests.len(), skill_message.content.clone())
        };
        assert_eq!(request_count, 1);
        assert!(skill_message_content.contains("review helper description"));
        assert!(skill_message_content.contains("Use review helper prompt content."));

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_runtime_prefers_configured_skill_context_source_over_filesystem_default() {
        let root = tempfile::tempdir().expect("tempdir");
        let storage_root = root.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/skills/filesystem-helper"))
            .expect("filesystem skill dir");
        std::fs::write(
            storage_root.join("system/skills/filesystem-helper/SKILL.md"),
            skill_md(
                "filesystem-helper",
                "filesystem helper description",
                "FILESYSTEM_HELPER_PROMPT_SENTINEL",
            ),
        )
        .expect("write filesystem skill");
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let gateway = Arc::new(RecordingGateway {
            reply: "configured skill context ok".to_string(),
            requests: Arc::clone(&requests),
        });
        let skill_source = Arc::new(StaticSkillContextSource::new(vec![
            HostSkillContextCandidate::loaded(
                skill_md(
                    "configured-helper",
                    "configured helper description",
                    "CONFIGURED_HELPER_PROMPT_SENTINEL",
                ),
                Some(SkillTrust::Trusted),
                Some(SkillVisibility::Visible),
            ),
        ]));
        let skill_context_source: Arc<dyn HostSkillContextSource> = skill_source;
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-skill-override-owner", storage_root)
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-skill-override-tenant".to_string(),
            agent_id: "runtime-skill-override-agent".to_string(),
            source_binding_id: "runtime-skill-override-source".to_string(),
            reply_target_binding_id: "runtime-skill-override-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_skill_context_source(skill_context_source)
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let reply = tokio::time::timeout(
            RUNTIME_SEND_TIMEOUT,
            runtime.send_user_message(&conversation, "review this"),
        )
        .await
        .expect("runtime send should finish")
        .expect("runtime send should succeed");

        assert_eq!(reply.status, TurnStatus::Completed);
        assert_eq!(reply.text.as_deref(), Some("configured skill context ok"));
        let combined_skill_context = {
            let requests = requests
                .lock()
                .expect("recording gateway requests lock poisoned");
            requests[0]
                .messages
                .iter()
                .filter(|message| {
                    message.role == HostManagedModelMessageRole::System
                        && message
                            .content_ref
                            .as_str()
                            .starts_with("msg:snippet.skill.")
                })
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert!(combined_skill_context.contains("configured helper description"));
        assert!(combined_skill_context.contains("CONFIGURED_HELPER_PROMPT_SENTINEL"));
        assert!(!combined_skill_context.contains("filesystem helper description"));
        assert!(!combined_skill_context.contains("FILESYSTEM_HELPER_PROMPT_SENTINEL"));

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_runtime_wires_filesystem_skills_by_default_to_model_calls() {
        let root = tempfile::tempdir().expect("tempdir");
        let storage_root = root.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/skills/system-helper"))
            .expect("system skill dir");
        std::fs::write(
            storage_root.join("system/skills/system-helper/SKILL.md"),
            skill_md(
                "system-helper",
                "system helper description",
                "SYSTEM_HELPER_PROMPT_SENTINEL",
            ),
        )
        .expect("write system skill");
        let local_helper_dir = user_skill_dir(
            &storage_root,
            "runtime-filesystem-skill-tenant",
            "runtime-filesystem-skill-owner",
            "local-helper",
        );
        std::fs::create_dir_all(&local_helper_dir).expect("user skill dir");
        std::fs::write(
            local_helper_dir.join("SKILL.md"),
            skill_md(
                "local-helper",
                "local helper description",
                "USER_HELPER_PROMPT_SENTINEL",
            ),
        )
        .expect("write user skill");
        std::fs::create_dir_all(storage_root.join("tenant-shared/skills/shared-helper"))
            .expect("tenant shared skill dir");
        std::fs::write(
            storage_root.join("tenant-shared/skills/shared-helper/SKILL.md"),
            skill_md(
                "shared-helper",
                "tenant shared helper description",
                "TENANT_SHARED_PROMPT_SENTINEL",
            ),
        )
        .expect("write tenant shared skill");
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let gateway = Arc::new(RecordingGateway {
            reply: "filesystem skill context ok".to_string(),
            requests: Arc::clone(&requests),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-filesystem-skill-owner", storage_root)
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-filesystem-skill-tenant".to_string(),
            agent_id: "runtime-filesystem-skill-agent".to_string(),
            source_binding_id: "runtime-filesystem-skill-source".to_string(),
            reply_target_binding_id: "runtime-filesystem-skill-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: RUNTIME_POLL_TIMEOUT,
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let reply = tokio::time::timeout(
            RUNTIME_SEND_TIMEOUT,
            runtime.send_user_message(&conversation, "/system-helper and /local-helper"),
        )
        .await
        .expect("runtime send should finish")
        .expect("runtime send should succeed");

        assert_eq!(reply.status, TurnStatus::Completed);
        assert_eq!(reply.text.as_deref(), Some("filesystem skill context ok"));
        let skill_messages = {
            let requests = requests
                .lock()
                .expect("recording gateway requests lock poisoned");
            requests[0]
                .messages
                .iter()
                .filter(|message| {
                    message.role == HostManagedModelMessageRole::System
                        && message
                            .content_ref
                            .as_str()
                            .starts_with("msg:snippet.skill.")
                })
                .map(|message| message.content.clone())
                .collect::<Vec<_>>()
        };
        let combined_skill_context = skill_messages.join("\n");
        assert_eq!(skill_messages.len(), 2);
        assert!(combined_skill_context.contains("system helper description"));
        assert!(combined_skill_context.contains("SYSTEM_HELPER_PROMPT_SENTINEL"));
        assert!(combined_skill_context.contains("local helper description"));
        assert!(combined_skill_context.contains("USER_HELPER_PROMPT_SENTINEL"));
        assert!(!combined_skill_context.contains("tenant shared helper description"));
        assert!(!combined_skill_context.contains("TENANT_SHARED_PROMPT_SENTINEL"));

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_runtime_backfills_legacy_owner_skill_root() {
        let root = tempfile::tempdir().expect("tempdir");
        let storage_root = root.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("skills/legacy-helper"))
            .expect("legacy skill dir");
        std::fs::write(
            storage_root.join("skills/legacy-helper/SKILL.md"),
            skill_md(
                "legacy-helper",
                "legacy helper description",
                "LEGACY_HELPER_PROMPT_SENTINEL",
            ),
        )
        .expect("write legacy helper skill");

        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-legacy-skill-owner", storage_root.clone())
                .with_runtime_policy(local_dev_runtime_policy()),
        );
        let runtime = build_reborn_runtime(input).await.expect("runtime");
        let conversation = runtime.new_conversation().await.expect("conversation");

        let result = runtime
            .execute_skill_message(&conversation, "$legacy-helper")
            .await
            .expect("execute skill message");

        assert_eq!(result.plan.activations().len(), 1);
        assert_eq!(result.plan.activations()[0].name, "legacy-helper");
        assert!(
            storage_root
                .join(
                    "tenants/reborn-cli/users/runtime-legacy-skill-owner/skills/legacy-helper/SKILL.md"
                )
                .exists()
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn execute_skill_message_returns_plan_and_reads_active_bundle_assets() {
        let root = tempfile::tempdir().expect("tempdir");
        let storage_root = root.path().join("local-dev");
        let asset_helper_dir = user_skill_dir(
            &storage_root,
            "runtime-skill-exec-tenant",
            "runtime-skill-exec-owner",
            "asset-helper",
        );
        std::fs::create_dir_all(asset_helper_dir.join("references"))
            .expect("asset skill references dir");
        std::fs::write(
            asset_helper_dir.join("SKILL.md"),
            skill_md(
                "asset-helper",
                "asset helper description",
                "ASSET_HELPER_PROMPT_SENTINEL",
            ),
        )
        .expect("write asset helper skill");
        std::fs::write(
            asset_helper_dir.join("references/policy.md"),
            "asset helper policy",
        )
        .expect("write asset helper policy");
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let gateway = Arc::new(RecordingGateway {
            reply: "asset helper ok".to_string(),
            requests: Arc::clone(&requests),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-skill-exec-owner", storage_root)
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-skill-exec-tenant".to_string(),
            agent_id: "runtime-skill-exec-agent".to_string(),
            source_binding_id: "runtime-skill-exec-source".to_string(),
            reply_target_binding_id: "runtime-skill-exec-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let result = tokio::time::timeout(
            RUNTIME_SEND_TIMEOUT,
            runtime.execute_skill_message(&conversation, "$asset-helper use policy"),
        )
        .await
        .expect("skill execution should finish")
        .expect("skill execution should succeed");

        assert_eq!(result.reply.status, TurnStatus::Completed);
        assert_eq!(result.reply.text.as_deref(), Some("asset helper ok"));
        assert_eq!(result.plan.activations().len(), 1);
        assert_eq!(result.plan.activations()[0].name, "asset-helper");
        assert_eq!(
            result.plan.activations()[0].source,
            Some(RebornSkillSourceKind::User)
        );
        assert_eq!(result.plan.active_bundles().len(), 1);
        assert_eq!(result.plan.active_bundles()[0].skill_name, "asset-helper");
        assert_eq!(
            result.plan.run_context().run_id,
            result.reply.run_id,
            "post-activation asset reads must reuse the real activation run context"
        );
        let asset = runtime
            .read_skill_execution_asset(
                &conversation,
                &result.plan,
                &result.plan.activations()[0],
                "references/policy.md",
            )
            .await
            .expect("active bundle asset read succeeds");

        assert_eq!(asset.skill_name, "asset-helper");
        assert_eq!(asset.path, "references/policy.md");
        assert_eq!(asset.into_utf8().unwrap(), "asset helper policy");

        let other_conversation = runtime
            .new_conversation()
            .await
            .expect("other conversation");
        let error = runtime
            .read_skill_execution_asset(
                &other_conversation,
                &result.plan,
                &result.plan.activations()[0],
                "references/policy.md",
            )
            .await
            .expect_err("plan should be bound to its activation conversation");
        assert!(
            error
                .to_string()
                .contains("skill execution plan does not belong to this conversation"),
            "unexpected error: {error}"
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_runtime_fails_closed_for_ambiguous_explicit_skill_before_model_call() {
        let root = tempfile::tempdir().expect("tempdir");
        let storage_root = root.path().join("local-dev");
        std::fs::create_dir_all(storage_root.join("system/skills/code-review"))
            .expect("system skill dir");
        std::fs::write(
            storage_root.join("system/skills/code-review/SKILL.md"),
            skill_md(
                "code-review",
                "system review description",
                "SYSTEM_REVIEW_PROMPT_SENTINEL",
            ),
        )
        .expect("write system skill");
        let user_code_review_dir = user_skill_dir(
            &storage_root,
            "runtime-ambiguous-skill-tenant",
            "runtime-ambiguous-skill-owner",
            "code-review",
        );
        std::fs::create_dir_all(&user_code_review_dir).expect("user skill dir");
        std::fs::write(
            user_code_review_dir.join("SKILL.md"),
            skill_md(
                "code-review",
                "user review description",
                "USER_REVIEW_PROMPT_SENTINEL",
            ),
        )
        .expect("write user skill");
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let gateway = Arc::new(RecordingGateway {
            reply: "should not reach model".to_string(),
            requests: Arc::clone(&requests),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-ambiguous-skill-owner", storage_root)
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-ambiguous-skill-tenant".to_string(),
            agent_id: "runtime-ambiguous-skill-agent".to_string(),
            source_binding_id: "runtime-ambiguous-skill-source".to_string(),
            reply_target_binding_id: "runtime-ambiguous-skill-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: RUNTIME_POLL_TIMEOUT,
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let reply = tokio::time::timeout(
            RUNTIME_SEND_TIMEOUT,
            runtime.send_user_message(&conversation, "/code-review this PR"),
        )
        .await
        .expect("runtime send should finish")
        .expect("runtime send should succeed");

        assert_ne!(reply.status, TurnStatus::Completed);
        assert!(
            requests
                .lock()
                .expect("recording gateway requests lock poisoned")
                .is_empty(),
            "ambiguous explicit skill should fail before model dispatch"
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_runtime_suppresses_explicit_setup_skill_when_workspace_marker_exists() {
        let root = tempfile::tempdir().expect("tempdir");
        let storage_root = root.path().join("local-dev");
        let marker_helper_dir = user_skill_dir(
            &storage_root,
            "runtime-setup-marker-tenant",
            "runtime-setup-marker-owner",
            "marker-helper",
        );
        std::fs::create_dir_all(&marker_helper_dir).expect("user skill dir");
        std::fs::create_dir_all(storage_root.join("workspace/markers")).expect("marker dir");
        std::fs::write(
            marker_helper_dir.join("SKILL.md"),
            skill_md_with_setup_marker(
                "marker-helper",
                "marker helper description",
                "markers/marker-helper.done",
                "MARKER_HELPER_PROMPT_SENTINEL",
            ),
        )
        .expect("write marker helper skill");
        std::fs::write(
            storage_root.join("workspace/markers/marker-helper.done"),
            "done",
        )
        .expect("write setup marker");
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let gateway = Arc::new(RecordingGateway {
            reply: "setup marker ok".to_string(),
            requests: Arc::clone(&requests),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-setup-marker-owner", storage_root)
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-setup-marker-tenant".to_string(),
            agent_id: "runtime-setup-marker-agent".to_string(),
            source_binding_id: "runtime-setup-marker-source".to_string(),
            reply_target_binding_id: "runtime-setup-marker-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: RUNTIME_POLL_TIMEOUT,
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let result = tokio::time::timeout(
            RUNTIME_SEND_TIMEOUT,
            runtime.execute_skill_message(&conversation, "$marker-helper"),
        )
        .await
        .expect("skill execution should finish")
        .expect("skill execution should succeed");

        assert_eq!(result.reply.status, TurnStatus::Completed);
        assert!(result.plan.activations().is_empty());
        let skill_messages = {
            let requests = requests
                .lock()
                .expect("recording gateway requests lock poisoned");
            requests[0]
                .messages
                .iter()
                .filter(|message| {
                    message.role == HostManagedModelMessageRole::System
                        && message
                            .content_ref
                            .as_str()
                            .starts_with("msg:snippet.skill.")
                })
                .count()
        };
        assert_eq!(skill_messages, 0);

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_runtime_activates_setup_skill_when_workspace_marker_is_absent() {
        let root = tempfile::tempdir().expect("tempdir");
        let storage_root = root.path().join("local-dev");
        let marker_helper_dir = user_skill_dir(
            &storage_root,
            "runtime-setup-marker-absent-tenant",
            "runtime-setup-marker-absent-owner",
            "marker-helper",
        );
        std::fs::create_dir_all(&marker_helper_dir).expect("user skill dir");
        std::fs::write(
            marker_helper_dir.join("SKILL.md"),
            skill_md_with_setup_marker(
                "marker-helper",
                "marker helper description",
                "markers/marker-helper.done",
                "MARKER_HELPER_PROMPT_SENTINEL",
            ),
        )
        .expect("write marker helper skill");
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let gateway = Arc::new(RecordingGateway {
            reply: "setup marker absent ok".to_string(),
            requests: Arc::clone(&requests),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-setup-marker-absent-owner", storage_root)
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-setup-marker-absent-tenant".to_string(),
            agent_id: "runtime-setup-marker-absent-agent".to_string(),
            source_binding_id: "runtime-setup-marker-absent-source".to_string(),
            reply_target_binding_id: "runtime-setup-marker-absent-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let result = tokio::time::timeout(
            RUNTIME_SEND_TIMEOUT,
            runtime.execute_skill_message(&conversation, "$marker-helper"),
        )
        .await
        .expect("skill execution should finish")
        .expect("skill execution should succeed");

        assert_eq!(result.reply.status, TurnStatus::Completed);
        assert_eq!(result.plan.activations().len(), 1);
        assert_eq!(result.plan.activations()[0].name, "marker-helper");
        let skill_context = {
            let requests = requests
                .lock()
                .expect("recording gateway requests lock poisoned");
            requests[0]
                .messages
                .iter()
                .filter(|message| {
                    message.role == HostManagedModelMessageRole::System
                        && message
                            .content_ref
                            .as_str()
                            .starts_with("msg:snippet.skill.")
                })
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert!(skill_context.contains("marker helper description"));
        assert!(skill_context.contains("MARKER_HELPER_PROMPT_SENTINEL"));

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_runtime_rejects_workspace_overlapping_default_skill_roots() {
        let root = tempfile::tempdir().expect("tempdir");
        let storage_root = root.path().join("local-dev");
        let workspace_root = storage_root.join("skills");
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let gateway = Arc::new(RecordingGateway {
            reply: "should not build".to_string(),
            requests,
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-overlap-owner", storage_root)
                .with_local_dev_workspace_root(workspace_root)
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-overlap-tenant".to_string(),
            agent_id: "runtime-overlap-agent".to_string(),
            source_binding_id: "runtime-overlap-source".to_string(),
            reply_target_binding_id: "runtime-overlap-reply".to_string(),
        })
        .with_model_gateway_override(gateway);

        let error = match build_reborn_runtime(input).await {
            Ok(runtime) => {
                runtime.shutdown().await.expect("runtime shutdown");
                panic!("overlapping workspace and skill roots should fail closed");
            }
            Err(error) => error,
        };

        assert!(
            error
                .to_string()
                .contains("must not overlap default skill root /skills"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn local_dev_runtime_skips_invalid_filesystem_skill_before_model_call() {
        let root = tempfile::tempdir().expect("tempdir");
        let storage_root = root.path().join("local-dev");
        let bad_helper_dir = user_skill_dir(
            &storage_root,
            "runtime-bad-skill-tenant",
            "runtime-bad-skill-owner",
            "bad-helper",
        );
        std::fs::create_dir_all(&bad_helper_dir).expect("bad skill dir");
        std::fs::write(
            bad_helper_dir.join("SKILL.md"),
            skill_md(
                "different-name",
                "bad helper description",
                "BAD_HELPER_PROMPT_SENTINEL",
            ),
        )
        .expect("write bad skill");
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let gateway = Arc::new(RecordingGateway {
            reply: "invalid skill skipped".to_string(),
            requests: Arc::clone(&requests),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-bad-skill-owner", storage_root)
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-bad-skill-tenant".to_string(),
            agent_id: "runtime-bad-skill-agent".to_string(),
            source_binding_id: "runtime-bad-skill-source".to_string(),
            reply_target_binding_id: "runtime-bad-skill-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let reply = tokio::time::timeout(
            RUNTIME_SEND_TIMEOUT,
            runtime.send_user_message(&conversation, "hello with no matching skill"),
        )
        .await
        .expect("runtime send should finish")
        .expect("runtime send should succeed");

        assert_eq!(reply.status, TurnStatus::Completed);
        assert_eq!(reply.text.as_deref(), Some("invalid skill skipped"));
        let combined_request_content = requests
            .lock()
            .expect("recording gateway requests lock poisoned")
            .iter()
            .flat_map(|request| request.messages.iter())
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!combined_request_content.contains("BAD_HELPER_PROMPT_SENTINEL"));

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_runtime_maps_workspace_to_configured_root() {
        let root = tempfile::tempdir().expect("tempdir");
        let workspace_root = tempfile::tempdir().expect("workspace tempdir");
        std::fs::write(
            workspace_root.path().join("workspace-sentinel.txt"),
            "visible through /workspace",
        )
        .expect("write sentinel");
        let gateway = Arc::new(WorkspaceListingGateway::default());
        let gateway_for_runtime: Arc<dyn HostManagedModelGateway> = gateway.clone();
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-workspace-owner", root.path().join("local-dev"))
                .with_local_dev_workspace_root(workspace_root.path().to_path_buf())
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-workspace-tenant".to_string(),
            agent_id: "runtime-workspace-agent".to_string(),
            source_binding_id: "runtime-workspace-source".to_string(),
            reply_target_binding_id: "runtime-workspace-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: RUNTIME_POLL_TIMEOUT,
        })
        .with_model_gateway_override(gateway_for_runtime);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let reply = tokio::time::timeout(
            RUNTIME_SEND_TIMEOUT,
            runtime.send_user_message(&conversation, "list workspace"),
        )
        .await
        .expect("runtime send should finish")
        .expect("runtime send should succeed");

        assert_eq!(reply.status, TurnStatus::Completed);
        assert_eq!(reply.text.as_deref(), Some("workspace ok"));
        let request_count = {
            let requests = gateway
                .requests
                .lock()
                .expect("workspace gateway requests lock poisoned");
            requests.len()
        };
        assert_eq!(
            request_count, 2,
            "workspace listing should require initial request plus tool-result follow-up"
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_runtime_webui_bundle_reuses_thread_and_turn_facades() {
        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(RecordingGateway {
            reply: "webui projection ok".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-webui-owner", root.path().join("local-dev"))
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-webui-tenant".to_string(),
            agent_id: "runtime-webui-agent".to_string(),
            source_binding_id: "runtime-webui-source".to_string(),
            reply_target_binding_id: "runtime-webui-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let runtime_turn_coordinator = runtime.webui_turn_coordinator();
        let bundle = build_webui_services(&runtime, None).expect("webui bundle");
        let caller = WebUiAuthenticatedCaller::new(
            TenantId::new("runtime-webui-tenant").unwrap(),
            UserId::new("runtime-webui-owner").unwrap(),
            Some(AgentId::new("runtime-webui-agent").unwrap()),
            None,
        );
        let created = bundle
            .api
            .create_thread(
                caller.clone(),
                WebUiCreateThreadRequest {
                    client_action_id: Some("create-webui-stream-thread".to_string()),
                    requested_thread_id: None,
                    project_id: None,
                },
            )
            .await
            .expect("create webui thread");
        let submitted = bundle
            .api
            .submit_turn(
                caller.clone(),
                WebUiSendMessageRequest {
                    client_action_id: Some("send-webui-stream-message".to_string()),
                    thread_id: Some(created.thread.thread_id.to_string()),
                    content: Some("hello webui stream".to_string()),
                    attachments: Vec::new(),
                },
            )
            .await
            .expect("submit webui turn");
        let RebornSubmitTurnResponse::Submitted { run_id, .. } = submitted else {
            panic!("webui submit should start a run");
        };
        let stream = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let stream = bundle
                    .api
                    .stream_events(
                        caller.clone(),
                        RebornStreamEventsRequest {
                            thread_id: created.thread.thread_id.to_string(),
                            after_cursor: None,
                        },
                    )
                    .await
                    .expect("webui event stream");
                if stream.events.iter().any(|event| {
                    matches!(
                        event.payload(),
                        ProductOutboundPayload::ProjectionSnapshot { state }
                            | ProductOutboundPayload::ProjectionUpdate { state }
                            if state.items.iter().any(|item| matches!(
                                item,
                                ProductProjectionItem::RunStatus {
                                    run_id: seen,
                                    status,
                                    ..
                                }
                                    if *seen == run_id && status == "completed"
                            ))
                    )
                }) {
                    break stream;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("completed webui projection should appear");

        let _api = bundle.api.clone();
        assert!(Arc::ptr_eq(
            &runtime_turn_coordinator,
            &runtime.webui_turn_coordinator()
        ));
        assert!(
            stream.events.iter().all(|event| matches!(
                event.payload(),
                ProductOutboundPayload::CapabilityActivity(_)
                    | ProductOutboundPayload::CapabilityDisplayPreview(_)
                    | ProductOutboundPayload::ProjectionSnapshot { .. }
                    | ProductOutboundPayload::ProjectionUpdate { .. }
            )),
            "webui bundle should expose only projection stream events"
        );
        assert_eq!(bundle.readiness, runtime.services().readiness);
        assert_eq!(bundle.readiness.state, RebornReadinessState::DevOnly);

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_webui_bundle_uses_local_lifecycle_facade_for_setup_extension() {
        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(RecordingGateway {
            reply: "webui lifecycle ok".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(
                "runtime-webui-lifecycle-owner",
                root.path().join("local-dev"),
            )
            .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-webui-lifecycle-tenant".to_string(),
            agent_id: "runtime-webui-lifecycle-agent".to_string(),
            source_binding_id: "runtime-webui-lifecycle-source".to_string(),
            reply_target_binding_id: "runtime-webui-lifecycle-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let bundle = build_webui_services(&runtime, None).expect("webui bundle");
        let caller = WebUiAuthenticatedCaller::new(
            TenantId::new("runtime-webui-lifecycle-tenant").unwrap(),
            UserId::new("runtime-webui-lifecycle-owner").unwrap(),
            Some(AgentId::new("runtime-webui-lifecycle-agent").unwrap()),
            None,
        );

        let setup = bundle
            .api
            .setup_extension(
                caller.clone(),
                LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github")
                    .expect("valid package ref"),
                WebUiSetupExtensionRequest::default(),
            )
            .await
            .expect("setup extension lifecycle projection");

        assert_eq!(setup.package_ref.id.as_str(), "github");
        assert_eq!(setup.phase, LifecyclePhase::Discovered);
        assert!(setup.blockers.is_empty());
        assert_eq!(setup.secrets.len(), 1);
        assert_eq!(setup.secrets[0].name, "github_runtime_token");
        assert_eq!(setup.secrets[0].provider, "github");
        assert!(!setup.secrets[0].optional);
        assert!(!setup.secrets[0].provided);
        assert!(matches!(
            setup.secrets[0].setup,
            RebornExtensionCredentialSetup::ManualToken
        ));
        let google_setup = bundle
            .api
            .setup_extension(
                caller.clone(),
                LifecyclePackageRef::new(LifecyclePackageKind::Extension, "google-calendar")
                    .expect("valid package ref"),
                WebUiSetupExtensionRequest::default(),
            )
            .await
            .expect("google setup extension lifecycle projection");
        assert_eq!(google_setup.secrets.len(), 1);
        let google_secret = &google_setup.secrets[0];
        assert_eq!(google_secret.provider, "google");
        assert!(!google_secret.provided);
        let RebornExtensionCredentialSetup::OAuth { scopes, .. } = &google_secret.setup else {
            panic!("Google setup secret should use OAuth")
        };
        assert_eq!(
            scopes
                .iter()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>(),
            [
                GOOGLE_CALENDAR_EVENTS_SCOPE.to_string(),
                GOOGLE_CALENDAR_READONLY_SCOPE.to_string(),
            ]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
        );
        let google_setup_json =
            serde_json::to_value(&google_setup.secrets[0]).expect("serialize setup secret");
        assert_eq!(google_setup_json["setup"]["kind"], "oauth");
        assert!(
            matches!(
                setup.payload.as_ref(),
                Some(LifecycleProductPayload::ExtensionList { extensions, count })
                    if *count == 1
                        && extensions.len() == 1
                        && extensions[0].summary.package_ref.id.as_str() == "github"
                        && extensions[0].summary.credential_requirements.len() == 1
            ),
            "local webui bundle should use the local lifecycle facade package projection"
        );
        assert!(
            !setup.blockers.iter().any(|blocker| matches!(
                blocker,
                LifecycleReadinessBlocker::Runtime { ref_id: Some(ref_id) }
                    if ref_id.as_str() == "reborn_lifecycle_facade_unwired"
            )),
            "local webui bundle must not fall back to the default unwired facade"
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_webui_bundle_exposes_outbound_preferences_facade() {
        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(RecordingGateway {
            reply: "webui outbound ok".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(
                "runtime-webui-outbound-owner",
                root.path().join("local-dev"),
            )
            .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-webui-outbound-tenant".to_string(),
            agent_id: "runtime-webui-outbound-agent".to_string(),
            source_binding_id: "runtime-webui-outbound-source".to_string(),
            reply_target_binding_id: "runtime-webui-outbound-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let bundle = build_webui_services(&runtime, None).expect("webui bundle");
        let caller = WebUiAuthenticatedCaller::new(
            TenantId::new("runtime-webui-outbound-tenant").unwrap(),
            UserId::new("runtime-webui-outbound-owner").unwrap(),
            Some(AgentId::new("runtime-webui-outbound-agent").unwrap()),
            None,
        );

        let cleared = bundle
            .api
            .set_outbound_preferences(
                caller.clone(),
                RebornSetOutboundPreferencesRequest {
                    final_reply_target_id: None,
                },
            )
            .await
            .expect("outbound preference clear uses composed facade");
        assert!(cleared.final_reply_target.is_none());

        let targets = bundle
            .api
            .list_outbound_delivery_targets(caller)
            .await
            .expect("outbound target listing uses composed facade");
        assert!(targets.targets.is_empty());

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[cfg(feature = "webui-v2-beta")]
    #[tokio::test]
    async fn webui_route_rejects_list_automations_without_agent_binding() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use ironclaw_webui_v2::{
            DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER, WebUiV2State, webui_v2_router,
        };
        use tower::ServiceExt;

        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(RecordingGateway {
            reply: "unused".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(
                "runtime-webui-no-agent-owner",
                root.path().join("local-dev"),
            )
            .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-webui-no-agent-tenant".to_string(),
            agent_id: "runtime-webui-no-agent-agent".to_string(),
            source_binding_id: "runtime-webui-no-agent-source".to_string(),
            reply_target_binding_id: "runtime-webui-no-agent-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_model_gateway_override(gateway);

        let mut runtime = build_reborn_runtime(input).await.expect("runtime builds");
        runtime.services.host_runtime = None;
        let bundle = build_webui_services(&runtime, None).expect("webui bundle");
        let caller_without_agent = WebUiAuthenticatedCaller::new(
            TenantId::new("runtime-webui-no-agent-tenant").unwrap(),
            UserId::new("runtime-webui-no-agent-owner").unwrap(),
            None,
            None,
        );
        let router = webui_v2_router(WebUiV2State::new(
            bundle.api,
            DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER,
        ))
        .layer(axum::Extension(caller_without_agent));

        let response = router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/webchat/v2/automations")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("route response");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[cfg(feature = "webui-v2-beta")]
    #[tokio::test]
    async fn open_reborn_identity_resolver_migrates_legacy_webui_identities_through_runtime() {
        use ironclaw_reborn_identity::{
            ExternalSubjectId, ProviderKind, ResolveExternalIdentity, SurfaceKind,
        };

        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(RecordingGateway {
            reply: "unused".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-identity-owner", root.path().join("local-dev"))
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-identity-tenant".to_string(),
            agent_id: "runtime-identity-agent".to_string(),
            source_binding_id: "runtime-identity-source".to_string(),
            reply_target_binding_id: "runtime-identity-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let tenant = TenantId::new("runtime-identity-tenant").expect("tenant");

        // Seed a legacy pre-#4381 WebUI identity into the SAME substrate DB the
        // runtime owns, exactly as the old store wrote it.
        let substrate = Arc::clone(
            runtime
                .services
                .local_runtime
                .as_ref()
                .expect("local runtime substrate")
                .identity_substrate_db
                .as_ref()
                .expect("libSQL identity substrate"),
        );
        let seed = substrate.connect().expect("substrate connection");
        seed.execute_batch(
            "CREATE TABLE user_identities (\
                 provider TEXT NOT NULL, provider_user_id TEXT NOT NULL, \
                 user_id TEXT NOT NULL, email TEXT, email_verified INTEGER NOT NULL, \
                 created_at TEXT NOT NULL, \
                 PRIMARY KEY (provider, provider_user_id));",
        )
        .await
        .expect("seed legacy schema");
        seed.execute(
            "INSERT INTO user_identities \
                 (provider, provider_user_id, user_id, email, email_verified, created_at) \
                 VALUES ('google', 'g-legacy', 'legacy-runtime-user', 'legacy@x.com', 1, \
                     '2026-01-01T00:00:00Z')",
            (),
        )
        .await
        .expect("seed legacy identity");
        // Drop the raw seed connection before the fold runs: production never
        // holds a second raw handle on the substrate, and an idle extra
        // connection here would contend with the filesystem-backed writes.
        drop(seed);

        // The production accessor `serve` relies on: it opens the resolver on
        // the runtime-owned substrate handle and runs the legacy fold, so the
        // returning legacy user must resolve to their original UserId rather
        // than being re-minted.
        let resolver = runtime
            .open_reborn_identity_resolver(&tenant)
            .await
            .expect("runtime carries a local-runtime substrate")
            .expect("resolver opens");
        let resolved = resolver
            .resolve_or_create(ResolveExternalIdentity {
                tenant_id: tenant.clone(),
                surface_kind: SurfaceKind::Oauth,
                provider_kind: ProviderKind::new("google").expect("provider"),
                provider_instance_id: None,
                external_subject_id: ExternalSubjectId::new("g-legacy").expect("subject"),
                email: Some("legacy@x.com".to_string()),
                email_verified: true,
                display_name: None,
            })
            .await
            .expect("resolve");
        assert_eq!(
            resolved.as_str(),
            "legacy-runtime-user",
            "a returning legacy SSO user keeps their UserId through the runtime accessor"
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[cfg(feature = "webui-v2-beta")]
    #[tokio::test]
    async fn webui_operator_diagnostics_route_exposes_composed_readiness_evidence() {
        use axum::body::{Body, to_bytes};
        use axum::http::{Request, StatusCode};
        use ironclaw_webui_v2::{
            DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER, WebUiV2Capabilities, WebUiV2State,
            webui_v2_router,
        };
        use tower::ServiceExt;

        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(RecordingGateway {
            reply: "unused".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(
                "runtime-webui-diagnostics-owner",
                root.path().join("local-dev"),
            )
            .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-webui-diagnostics-tenant".to_string(),
            agent_id: "runtime-webui-diagnostics-agent".to_string(),
            source_binding_id: "runtime-webui-diagnostics-source".to_string(),
            reply_target_binding_id: "runtime-webui-diagnostics-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let bundle = build_webui_services(&runtime, None).expect("webui bundle");
        let caller = WebUiAuthenticatedCaller::new(
            TenantId::new("runtime-webui-diagnostics-tenant").unwrap(),
            UserId::new("runtime-webui-diagnostics-owner").unwrap(),
            Some(AgentId::new("runtime-webui-diagnostics-agent").unwrap()),
            None,
        );
        let router = webui_v2_router(WebUiV2State::new(
            bundle.api,
            DEFAULT_SSE_MAX_CONCURRENT_PER_CALLER,
        ))
        .layer(axum::Extension(WebUiV2Capabilities {
            operator_webui_config: true,
        }))
        .layer(axum::Extension(caller));

        let response = router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/webchat/v2/operator/diagnostics")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("route response");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body bytes");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("diagnostics json");
        assert!(
            json["operator_status"]["checks"]
                .as_array()
                .expect("status checks")
                .iter()
                .any(|check| check["id"] == "readiness_composition_profile"
                    && check["status"] == "blocked"
                    && check["summary"]
                        .as_str()
                        .is_some_and(|summary| summary.contains("reason=dev-only-profile"))),
            "diagnostics route should expose readiness-derived status checks: {json}"
        );
        assert!(
            json["diagnostics"]
                .as_array()
                .expect("diagnostics")
                .iter()
                .any(|diagnostic| diagnostic["reason_code"]
                    == "operator_doctor_readiness_composition_profile_blocked"
                    && diagnostic["key"] == "readiness_composition_profile"),
            "diagnostics route should expose readiness-derived doctor diagnostics: {json}"
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[cfg(feature = "webui-v2-beta")]
    #[tokio::test]
    async fn open_reborn_identity_resolver_migrates_legacy_verified_email_linking() {
        use ironclaw_reborn_identity::{
            ExternalSubjectId, ProviderKind, ResolveExternalIdentity, SurfaceKind,
        };

        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(RecordingGateway {
            reply: "unused".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(
                "runtime-identity-link-owner",
                root.path().join("local-dev"),
            )
            .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-identity-link-tenant".to_string(),
            agent_id: "runtime-identity-link-agent".to_string(),
            source_binding_id: "runtime-identity-link-source".to_string(),
            reply_target_binding_id: "runtime-identity-link-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let tenant = TenantId::new("runtime-identity-link-tenant").expect("tenant");

        // Seed a legacy pre-#4381 WebUI Google identity with a VERIFIED email.
        let substrate = Arc::clone(
            runtime
                .services
                .local_runtime
                .as_ref()
                .expect("local runtime substrate")
                .identity_substrate_db
                .as_ref()
                .expect("libSQL identity substrate"),
        );
        let seed = substrate.connect().expect("substrate connection");
        seed.execute_batch(
            "CREATE TABLE user_identities (\
                 provider TEXT NOT NULL, provider_user_id TEXT NOT NULL, \
                 user_id TEXT NOT NULL, email TEXT, email_verified INTEGER NOT NULL, \
                 created_at TEXT NOT NULL, \
                 PRIMARY KEY (provider, provider_user_id));",
        )
        .await
        .expect("seed legacy schema");
        seed.execute(
            "INSERT INTO user_identities \
                 (provider, provider_user_id, user_id, email, email_verified, created_at) \
                 VALUES ('google', 'g-legacy', 'legacy-link-user', 'shared@x.com', 1, \
                     '2026-01-01T00:00:00Z')",
            (),
        )
        .await
        .expect("seed legacy identity");
        drop(seed);

        // The fold must seed the canonical verified-email index from the
        // migrated row's verified email — not just preserve the per-subject id.
        let resolver = runtime
            .open_reborn_identity_resolver(&tenant)
            .await
            .expect("runtime carries a local-runtime substrate")
            .expect("resolver opens");

        // The upgrade case: a LATER login through a DIFFERENT OAuth provider
        // with the SAME verified email must link to the migrated user instead
        // of minting a second one.
        let via_github = resolver
            .resolve_or_create(ResolveExternalIdentity {
                tenant_id: tenant.clone(),
                surface_kind: SurfaceKind::Oauth,
                provider_kind: ProviderKind::new("github").expect("provider"),
                provider_instance_id: None,
                external_subject_id: ExternalSubjectId::new("gh-new").expect("subject"),
                email: Some("shared@x.com".to_string()),
                email_verified: true,
                display_name: None,
            })
            .await
            .expect("resolve");
        assert_eq!(
            via_github.as_str(),
            "legacy-link-user",
            "a migrated verified legacy email must link a later different-provider login"
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn build_webui_services_without_local_runtime_returns_503_on_list_automations() {
        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(RecordingGateway {
            reply: "unused".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(
                "runtime-webui-no-host-owner",
                root.path().join("local-dev"),
            )
            .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-webui-no-host-tenant".to_string(),
            agent_id: "runtime-webui-no-host-agent".to_string(),
            source_binding_id: "runtime-webui-no-host-source".to_string(),
            reply_target_binding_id: "runtime-webui-no-host-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_model_gateway_override(gateway);

        let mut runtime = build_reborn_runtime(input).await.expect("runtime builds");
        runtime.services.local_runtime = None;
        let bundle = build_webui_services(&runtime, None).expect("webui bundle");
        let caller = WebUiAuthenticatedCaller::new(
            TenantId::new("runtime-webui-no-host-tenant").unwrap(),
            UserId::new("runtime-webui-no-host-owner").unwrap(),
            Some(AgentId::new("runtime-webui-no-host-agent").unwrap()),
            None,
        );

        let error = bundle
            .api
            .list_automations(caller, WebUiListAutomationsRequest::default())
            .await
            .expect_err("missing host runtime should leave automation facade unavailable");

        assert_eq!(error.code, RebornServicesErrorCode::Unavailable);
        assert_eq!(error.kind, RebornServicesErrorKind::ServiceUnavailable);
        assert_eq!(error.status_code, 503);
        assert!(error.retryable);
        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_webui_setup_extension_stores_and_rotates_runtime_credentials() {
        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(RecordingGateway {
            reply: "webui lifecycle ok".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(
                "runtime-webui-credential-owner",
                root.path().join("local-dev"),
            )
            .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-webui-credential-tenant".to_string(),
            agent_id: "runtime-webui-credential-agent".to_string(),
            source_binding_id: "runtime-webui-credential-source".to_string(),
            reply_target_binding_id: "runtime-webui-credential-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let bundle = build_webui_services(&runtime, None).expect("webui bundle");
        let caller = WebUiAuthenticatedCaller::new(
            TenantId::new("runtime-webui-credential-tenant").unwrap(),
            UserId::new("runtime-webui-credential-owner").unwrap(),
            Some(AgentId::new("runtime-webui-credential-agent").unwrap()),
            None,
        );
        let package_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github").unwrap();

        let first = bundle
            .api
            .setup_extension(
                caller.clone(),
                package_ref.clone(),
                WebUiSetupExtensionRequest {
                    action: Some("submit".to_string()),
                    payload: Some(serde_json::json!({
                        "secrets": {
                            "github_runtime_token": "ghp_first_token"
                        },
                        "fields": {}
                    })),
                },
            )
            .await
            .expect("submit github runtime token");
        assert_eq!(first.secrets.len(), 1);
        assert!(first.secrets[0].provided);
        let first_credential_ref = first.secrets[0]
            .credential_ref
            .clone()
            .expect("credential ref");

        let second = bundle
            .api
            .setup_extension(
                caller,
                package_ref,
                WebUiSetupExtensionRequest {
                    action: Some("submit".to_string()),
                    payload: Some(serde_json::json!({
                        "secrets": {
                            "github_runtime_token": "ghp_second_token"
                        },
                        "fields": {}
                    })),
                },
            )
            .await
            .expect("rotate github runtime token");
        assert_eq!(second.secrets.len(), 1);
        assert!(second.secrets[0].provided);
        assert_eq!(
            second.secrets[0].credential_ref.as_deref(),
            Some(first_credential_ref.as_str()),
            "reconfigure should rotate the existing account instead of creating a duplicate"
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_webui_bundle_routes_approval_gates_into_interaction_service() {
        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(RecordingGateway {
            reply: "unused".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(
                "runtime-webui-approval-owner",
                root.path().join("local-dev"),
            )
            .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-webui-approval-tenant".to_string(),
            agent_id: "runtime-webui-approval-agent".to_string(),
            source_binding_id: "runtime-webui-approval-source".to_string(),
            reply_target_binding_id: "runtime-webui-approval-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let bundle = build_webui_services(&runtime, None).expect("webui bundle");
        let caller = WebUiAuthenticatedCaller::new(
            TenantId::new("runtime-webui-approval-tenant").unwrap(),
            UserId::new("runtime-webui-approval-owner").unwrap(),
            Some(AgentId::new("runtime-webui-approval-agent").unwrap()),
            None,
        );
        let created = bundle
            .api
            .create_thread(
                caller.clone(),
                WebUiCreateThreadRequest {
                    client_action_id: Some("create-webui-approval-thread".to_string()),
                    requested_thread_id: None,
                    project_id: None,
                },
            )
            .await
            .expect("create thread");
        let gate_ref = approval_gate_ref(ApprovalRequestId::new()).expect("approval gate");

        let err = bundle
            .api
            .resolve_gate(
                caller,
                WebUiResolveGateRequest {
                    client_action_id: Some("resolve-webui-approval-gate".to_string()),
                    thread_id: Some(created.thread.thread_id.to_string()),
                    run_id: Some(TurnRunId::new().to_string()),
                    gate_ref: Some(gate_ref.as_str().to_string()),
                    resolution: Some("approved".to_string()),
                    always: None,
                    credential_ref: None,
                },
            )
            .await
            .expect_err("missing approval gate should reach approval interaction service");

        assert_eq!(err.code, RebornServicesErrorCode::NotFound);
        assert_eq!(err.kind, RebornServicesErrorKind::NotFound);
        assert_eq!(err.status_code, 404);
        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_webui_bundle_routes_auth_gates_into_interaction_service() {
        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(RecordingGateway {
            reply: "unused".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-webui-auth-owner", root.path().join("local-dev"))
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-webui-auth-tenant".to_string(),
            agent_id: "runtime-webui-auth-agent".to_string(),
            source_binding_id: "runtime-webui-auth-source".to_string(),
            reply_target_binding_id: "runtime-webui-auth-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let bundle = build_webui_services(&runtime, None).expect("webui bundle");
        let caller = WebUiAuthenticatedCaller::new(
            TenantId::new("runtime-webui-auth-tenant").unwrap(),
            UserId::new("runtime-webui-auth-owner").unwrap(),
            Some(AgentId::new("runtime-webui-auth-agent").unwrap()),
            None,
        );
        let created = bundle
            .api
            .create_thread(
                caller.clone(),
                WebUiCreateThreadRequest {
                    client_action_id: Some("create-webui-auth-thread".to_string()),
                    requested_thread_id: None,
                    project_id: None,
                },
            )
            .await
            .expect("create thread");

        let err = bundle
            .api
            .resolve_gate(
                caller,
                WebUiResolveGateRequest {
                    client_action_id: Some("resolve-webui-auth-gate".to_string()),
                    thread_id: Some(created.thread.thread_id.to_string()),
                    run_id: Some(TurnRunId::new().to_string()),
                    gate_ref: Some("gate:hook-auth-missing".to_string()),
                    resolution: Some("denied".to_string()),
                    always: None,
                    credential_ref: None,
                },
            )
            .await
            .expect_err("missing auth gate should reach auth interaction service");

        assert_eq!(err.code, RebornServicesErrorCode::NotFound);
        assert_eq!(err.kind, RebornServicesErrorKind::BlockedAuthentication);
        assert_eq!(err.status_code, 404);
        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_webui_spawn_approval_emits_redacted_audit_and_grants_process() {
        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(RecordingGateway {
            reply: "unused".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-webui-audit-owner", root.path().join("local-dev"))
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-webui-audit-tenant".to_string(),
            agent_id: "runtime-webui-audit-agent".to_string(),
            source_binding_id: "runtime-webui-audit-source".to_string(),
            reply_target_binding_id: "runtime-webui-audit-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let bundle = build_webui_services(&runtime, None).expect("webui bundle");
        let caller = WebUiAuthenticatedCaller::new(
            TenantId::new("runtime-webui-audit-tenant").unwrap(),
            UserId::new("runtime-webui-audit-owner").unwrap(),
            Some(AgentId::new("runtime-webui-audit-agent").unwrap()),
            None,
        );
        let created = bundle
            .api
            .create_thread(
                caller.clone(),
                WebUiCreateThreadRequest {
                    client_action_id: Some("create-webui-audit-thread".to_string()),
                    requested_thread_id: None,
                    project_id: None,
                },
            )
            .await
            .expect("create thread");
        let scope = caller.turn_scope(created.thread.thread_id.clone());
        let actor = caller.actor();
        let submitted = runtime
            .turn_coordinator
            .submit_turn(SubmitTurnRequest {
                scope: scope.clone(),
                actor: actor.clone(),
                accepted_message_ref: AcceptedMessageRef::new("msg:audit").unwrap(),
                source_binding_ref: SourceBindingRef::new("src:audit").unwrap(),
                reply_target_binding_ref: ReplyTargetBindingRef::new("reply:audit").unwrap(),
                requested_run_profile: None,
                idempotency_key: IdempotencyKey::new("submit-audit").unwrap(),
                received_at: chrono::Utc::now(),
                requested_run_id: None,
                parent_run_id: None,
                subagent_depth: 0,
                spawn_tree_root_run_id: None,
                product_context: None,
            })
            .await
            .expect("submit turn");
        let run_id = match submitted {
            SubmitTurnResponse::Accepted { run_id, .. } => run_id,
        };
        let local_runtime = runtime
            .services
            .local_runtime
            .as_ref()
            .expect("local runtime services");
        let runner_id = TurnRunnerId::new();
        let lease_token = TurnLeaseToken::new();
        let claimed = local_runtime
            .turn_state
            .claim_next_run(ClaimRunRequest {
                runner_id,
                lease_token,
                scope_filter: Some(scope.clone()),
            })
            .await
            .expect("claim run")
            .expect("claimed run");
        assert_eq!(claimed.state.run_id, run_id);
        let request_id = ApprovalRequestId::new();
        let gate_ref = approval_gate_ref(request_id).expect("approval gate");
        local_runtime
            .turn_state
            .block_run(BlockRunRequest {
                run_id,
                runner_id,
                lease_token,
                checkpoint_id: TurnCheckpointId::new(),
                state_ref: LoopCheckpointStateRef::new("checkpoint:audit").unwrap(),
                reason: BlockedReason::Approval {
                    gate_ref: gate_ref.clone(),
                },
            })
            .await
            .expect("block approval");
        let resource_scope = ResourceScope {
            tenant_id: scope.tenant_id.clone(),
            user_id: actor.user_id.clone(),
            agent_id: scope.agent_id.clone(),
            project_id: scope.project_id.clone(),
            mission_id: None,
            thread_id: Some(scope.thread_id.clone()),
            invocation_id: InvocationId::new(),
        };
        let capability = CapabilityId::new("demo.echo").expect("capability");
        let mut approval = ApprovalRequest {
            id: request_id,
            correlation_id: CorrelationId::new(),
            requested_by: Principal::User(actor.user_id.clone()),
            action: Box::new(Action::SpawnCapability {
                capability: capability.clone(),
                estimated_resources: ResourceEstimate::default(),
            }),
            invocation_fingerprint: None,
            reason: "raw /Users/alice/private token sk-live".to_string(),
            reusable_scope: None,
        };
        approval.invocation_fingerprint = Some(
            InvocationFingerprint::for_spawn(
                &resource_scope,
                &capability,
                &ResourceEstimate::default(),
                &serde_json::json!({"secret": "hidden"}),
            )
            .expect("fingerprint"),
        );
        local_runtime
            .approval_requests
            .save_pending(resource_scope.clone(), approval)
            .await
            .expect("save approval");
        let streamed = bundle
            .api
            .stream_events(
                caller.clone(),
                RebornStreamEventsRequest {
                    thread_id: scope.thread_id.to_string(),
                    after_cursor: None,
                },
            )
            .await
            .expect("approval gate event stream");
        assert!(
            streamed.events.iter().any(|event| {
                matches!(
                    event.payload(),
                    ProductOutboundPayload::GatePrompt(prompt)
                        if prompt.turn_run_id == run_id
                            && prompt.gate_ref == gate_ref.as_str()
                            && prompt.headline == "Approval required"
                )
            }),
            "blocked approval run should be visible as a gate prompt on the product event stream"
        );

        bundle
            .api
            .resolve_gate(
                caller,
                WebUiResolveGateRequest {
                    client_action_id: Some("resolve-webui-audit-gate".to_string()),
                    thread_id: Some(scope.thread_id.to_string()),
                    run_id: Some(run_id.to_string()),
                    gate_ref: Some(gate_ref.as_str().to_string()),
                    resolution: Some("approved".to_string()),
                    always: None,
                    credential_ref: None,
                },
            )
            .await
            .expect("resolve approval gate");

        let records = runtime.webui_approval_audit_sink().records();
        assert_eq!(records.len(), 1);
        let serialized = serde_json::to_string(&records[0]).expect("serialize audit");
        for forbidden in ["/Users/alice/private", "sk-live", "hidden", "sha256:"] {
            assert!(
                !serialized.contains(forbidden),
                "approval audit leaked {forbidden}: {serialized}"
            );
        }
        let leases = local_runtime
            .capability_leases
            .leases_for_scope(&resource_scope)
            .await;
        assert_eq!(leases.len(), 1);
        assert_eq!(
            leases[0].grant.issued_by,
            Principal::User(actor.user_id.clone()),
            "product approval service should stamp the approving user on the resume lease"
        );
        assert!(
            leases[0]
                .grant
                .constraints
                .allowed_effects
                .contains(&EffectKind::SpawnProcess)
        );
        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn local_dev_webui_bundle_records_selectable_filesystem_skill_context() {
        let root = tempfile::tempdir().expect("tempdir");
        let storage_root = root.path().join("local-dev");
        let webui_helper_dir = user_skill_dir(
            &storage_root,
            "runtime-webui-skill-tenant",
            "runtime-webui-skill-user",
            "webui-helper",
        );
        std::fs::create_dir_all(&webui_helper_dir).expect("user skill dir");
        std::fs::write(
            webui_helper_dir.join("SKILL.md"),
            skill_md(
                "webui-helper",
                "webui helper description",
                "WEBUI_HELPER_PROMPT_SENTINEL",
            ),
        )
        .expect("write user skill");
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let gateway = Arc::new(RecordingGateway {
            reply: "webui skill context ok".to_string(),
            requests: Arc::clone(&requests),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("runtime-webui-skill-owner", storage_root)
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-webui-skill-tenant".to_string(),
            agent_id: "runtime-webui-skill-agent".to_string(),
            source_binding_id: "runtime-webui-skill-source".to_string(),
            reply_target_binding_id: "runtime-webui-skill-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let bundle = build_webui_services(&runtime, None).expect("webui bundle");
        let webui_user_id = UserId::new("runtime-webui-skill-user").unwrap();
        let caller = WebUiAuthenticatedCaller::new(
            TenantId::new("runtime-webui-skill-tenant").unwrap(),
            webui_user_id.clone(),
            Some(AgentId::new("runtime-webui-skill-agent").unwrap()),
            None,
        );
        let created = bundle
            .api
            .create_thread(
                caller.clone(),
                WebUiCreateThreadRequest {
                    client_action_id: Some("create-webui-skill-thread".to_string()),
                    requested_thread_id: None,
                    project_id: None,
                },
            )
            .await
            .expect("create thread");
        let submitted = bundle
            .api
            .submit_turn(
                caller,
                WebUiSendMessageRequest {
                    client_action_id: Some("send-webui-skill-message".to_string()),
                    thread_id: Some(created.thread.thread_id.to_string()),
                    content: Some("$webui-helper please help".to_string()),
                    attachments: Vec::new(),
                },
            )
            .await
            .expect("submit turn");
        let RebornSubmitTurnResponse::Submitted {
            thread_id,
            accepted_message_ref,
            ..
        } = submitted
        else {
            panic!("webui submit should start a run");
        };
        let resolved_run_profile = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("resolve run profile");
        let source = runtime
            .webui_skill_activation_source()
            .expect("webui skill activation source");
        let turn_scope = TurnScope::new_with_owner(
            TenantId::new("runtime-webui-skill-tenant").unwrap(),
            Some(AgentId::new("runtime-webui-skill-agent").unwrap()),
            None,
            thread_id.clone(),
            Some(webui_user_id.clone()),
        );
        let context = LoopRunContext::new(
            turn_scope,
            TurnId::new(),
            TurnRunId::new(),
            resolved_run_profile,
        )
        .with_accepted_message_ref(accepted_message_ref)
        .with_actor(TurnActor::new(webui_user_id));
        let selected = source
            .load_skill_context_candidates(&context)
            .await
            .expect("webui-recorded skill context should load");
        let combined_skill_context = selected
            .iter()
            .map(|candidate| candidate.loaded_skill_md().unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(selected.len(), 1);
        assert!(combined_skill_context.contains("webui helper description"));
        assert!(combined_skill_context.contains("WEBUI_HELPER_PROMPT_SENTINEL"));

        runtime.shutdown().await.expect("runtime shutdown");
    }

    /// Multi-call model response with a mid-register surface change must not kill the run.
    ///
    /// Scenario: the scripted gateway (a) registers tool call #1, (b) activates an extension
    /// (deterministic surface-content change), (c) registers tool call #2, then returns both
    /// candidates together.  Before the fix, register #2 rebuilt the inner port, wiping the
    /// snapshot that candidate #1 referred to; the executor hit StaleSurface on the first
    /// candidate and collapsed to a terminal HostUnavailable failure.  After the fix, both
    /// candidates carry the same (prompt-stage) surface version and the run completes.
    #[tokio::test]
    async fn multi_tool_call_response_survives_surface_change_mid_register() {
        use ironclaw_product_workflow::{
            LifecycleProductAction, LifecycleProductContext, LifecycleProductFacade,
            LifecycleProductSurfaceContext,
        };
        use std::sync::OnceLock;

        // Gateway state seeded after runtime build.
        struct LifecycleFacadeHandle {
            facade: crate::lifecycle::RebornLocalLifecycleFacade,
        }

        impl std::fmt::Debug for LifecycleFacadeHandle {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct("LifecycleFacadeHandle").finish()
            }
        }

        struct MultiToolCallGateway {
            calls: StdMutex<usize>,
            facade_slot: Arc<OnceLock<LifecycleFacadeHandle>>,
        }

        #[async_trait]
        impl HostManagedModelGateway for MultiToolCallGateway {
            async fn stream_model(
                &self,
                _request: HostManagedModelRequest,
            ) -> Result<HostManagedModelResponse, HostManagedModelError> {
                Err(HostManagedModelError::safe(
                    HostManagedModelErrorKind::InvalidRequest,
                    "expected capability-aware model path",
                ))
            }

            async fn stream_model_with_capabilities(
                &self,
                _request: HostManagedModelRequest,
                capabilities: Arc<dyn ironclaw_turns::run_profile::LoopCapabilityPort>,
            ) -> Result<HostManagedModelResponse, HostManagedModelError> {
                let call_index = {
                    let mut calls = self.calls.lock().expect("multi-tool gateway lock poisoned");
                    let idx = *calls;
                    *calls += 1;
                    idx
                };

                if call_index > 0 {
                    // Second model call: capability results have been fed back — finish the run.
                    return Ok(HostManagedModelResponse::assistant_reply(
                        "multi-tool surface-change ok",
                    ));
                }

                // ── First model call ──────────────────────────────────────────────────
                // Trigger prompt-stage surface snapshot (establishes V1).
                capabilities
                    .visible_capabilities(VisibleCapabilityRequest)
                    .await
                    .map_err(model_capability_error)?;

                // Find the builtin echo tool.
                let echo_id =
                    ironclaw_host_api::CapabilityId::new("builtin.echo").expect("echo id");
                let echo_tool = capabilities
                    .tool_definitions()
                    .map_err(model_capability_error)?
                    .into_iter()
                    .find(|def| def.capability_id == echo_id)
                    .expect("echo provider tool definition");

                // Register call #1 — candidate carries surface version V1.
                let mut call1 = ProviderToolCall {
                    provider_id: "test-provider".to_string(),
                    provider_model_id: "test-model".to_string(),
                    turn_id: Some("provider-turn-multi".to_string()),
                    id: "call-multi-1".to_string(),
                    name: echo_tool.name.clone(),
                    arguments: serde_json::json!({"message": "hello from call 1"}),
                    response_reasoning: None,
                    reasoning: None,
                    signature: None,
                };
                let candidate1 = capabilities
                    .register_provider_tool_call(call1.clone())
                    .await
                    .map_err(model_capability_error)?;

                // Activate the github extension — deterministic surface-content change.
                // Pre-fix: this rebuilds the inner port, wiping candidate1's snapshot.
                let facade_handle = self
                    .facade_slot
                    .get()
                    .expect("lifecycle facade must be seeded before send_user_message");
                let package_ref =
                    LifecyclePackageRef::new(LifecyclePackageKind::Extension, "github")
                        .expect("valid github ref");
                let ctx = LifecycleProductContext::Surface(LifecycleProductSurfaceContext {
                    tenant_id: TenantId::new("tenant-multi-tool-surface").expect("tenant id"),
                    user_id: UserId::new("user-multi-tool-surface").expect("user id"),
                    agent_id: None,
                    project_id: None,
                });
                facade_handle
                    .facade
                    .execute(
                        ctx.clone(),
                        LifecycleProductAction::ExtensionInstall {
                            package_ref: package_ref.clone(),
                        },
                    )
                    .await
                    .expect("install github extension");
                facade_handle
                    .facade
                    .execute(
                        ctx,
                        LifecycleProductAction::ExtensionActivate { package_ref },
                    )
                    .await
                    .expect("activate github extension");

                // Register call #2 — after surface change.
                // Post-fix: reuses current port, so both candidates carry the same surface version.
                call1.id = "call-multi-2".to_string();
                call1.arguments = serde_json::json!({"message": "hello from call 2"});
                let candidate2 = capabilities
                    .register_provider_tool_call(call1)
                    .await
                    .map_err(model_capability_error)?;

                // Both candidates must carry the same surface version after the fix.
                // (We cannot assert this here without breaking the pre-fix path,
                //  so we rely on the run-completion assertion in the test body.)
                Ok(HostManagedModelResponse::capability_calls(
                    vec![candidate1, candidate2],
                    "",
                ))
            }
        }

        // ── Test body ──────────────────────────────────────────────────────────────
        let root = tempfile::tempdir().expect("tempdir");
        let facade_slot: Arc<OnceLock<LifecycleFacadeHandle>> = Arc::new(OnceLock::new());
        let gateway = Arc::new(MultiToolCallGateway {
            calls: StdMutex::new(0),
            facade_slot: Arc::clone(&facade_slot),
        });
        let gateway_for_runtime: Arc<dyn HostManagedModelGateway> = gateway;

        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(
                "runtime-multi-tool-surface-owner",
                root.path().join("local-dev"),
            )
            .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-multi-tool-surface-tenant".to_string(),
            agent_id: "runtime-multi-tool-surface-agent".to_string(),
            source_binding_id: "runtime-multi-tool-surface-source".to_string(),
            reply_target_binding_id: "runtime-multi-tool-surface-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: RUNTIME_POLL_TIMEOUT,
        })
        .with_model_gateway_override(gateway_for_runtime);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");

        // Seed the lifecycle facade before the model gateway runs.
        let local_runtime = runtime
            .services
            .local_runtime
            .as_ref()
            .expect("local runtime substrate");
        let extension_management = local_runtime
            .extension_management
            .as_ref()
            .expect("extension management")
            .clone();
        let facade = crate::lifecycle::RebornLocalLifecycleFacade::new(
            local_runtime.skill_management.clone(),
        )
        .with_extension_management(extension_management)
        .with_runtime_credential_accounts(Arc::new(MultiToolConfiguredCredentials));
        facade_slot
            .set(LifecycleFacadeHandle { facade })
            .expect("facade slot should be empty before seeding");

        let conversation = runtime.new_conversation().await.expect("conversation");
        let reply = tokio::time::timeout(
            RUNTIME_SEND_TIMEOUT,
            runtime.send_user_message(&conversation, "use echo tool twice"),
        )
        .await
        .expect("runtime send should finish within timeout")
        .expect("runtime send should succeed");

        assert_eq!(
            reply.status,
            TurnStatus::Completed,
            "multi-tool response with mid-register surface change must not produce terminal failure; status={:?} text={:?}",
            reply.status,
            reply.text,
        );
        assert_eq!(reply.text.as_deref(), Some("multi-tool surface-change ok"));

        runtime.shutdown().await.expect("runtime shutdown");
    }

    /// Regression guard: a message that arrives while the thread is busy is stored with
    /// `RejectedBusy` status and must NOT be auto-resubmitted when the blocking run
    /// reaches a terminal state.
    ///
    /// Scenario:
    ///  A – submitted via `turn_coordinator.submit_turn`; worker is stopped so it stays
    ///      Queued and holds the active-lock.
    ///  B – submitted via `bundle.api.submit_turn` (WebUI path); thread is busy → stored
    ///      as `RejectedBusy`; response carries a non-empty `notice`.
    ///  Cancel A → B stays `RejectedBusy` (no auto-resubmission).
    ///  C – submitted after A is cancelled; thread is free → `Submitted`.
    ///
    /// arch-note: lives in runtime.rs (adds ~200 lines to an already >3000-line file) because
    /// it requires `build_reborn_runtime` + full turn-runner control that only the runtime test
    /// harness provides; moving it would require duplicating that harness. Decomposition of
    /// runtime.rs is tracked in plan #4471.
    #[tokio::test]
    async fn rejected_busy_message_not_auto_resubmitted_after_run_cancellation() {
        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(RecordingGateway {
            reply: "busy-drain ok".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(
                "runtime-rejected-busy-owner",
                root.path().join("local-dev"),
            )
            .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-rejected-busy-tenant".to_string(),
            agent_id: "runtime-rejected-busy-agent".to_string(),
            source_binding_id: "runtime-rejected-busy-source".to_string(),
            reply_target_binding_id: "runtime-rejected-busy-reply".to_string(),
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        // Stop the worker so run A stays Queued and holds the thread active-lock.
        stop_turn_runner_worker_for_manual_state_test(&runtime).await;

        let bundle = build_webui_services(&runtime, None).expect("webui bundle");
        let caller = WebUiAuthenticatedCaller::new(
            TenantId::new("runtime-rejected-busy-tenant").unwrap(),
            UserId::new("runtime-rejected-busy-owner").unwrap(),
            Some(AgentId::new("runtime-rejected-busy-agent").unwrap()),
            None,
        );

        // Create the thread via WebUI so the thread record exists.
        let created = bundle
            .api
            .create_thread(
                caller.clone(),
                WebUiCreateThreadRequest {
                    client_action_id: Some("create-rejected-busy-thread".to_string()),
                    requested_thread_id: None,
                    project_id: None,
                },
            )
            .await
            .expect("create thread");
        let thread_id = created.thread.thread_id.clone();

        // Submit message A directly so we hold the active-lock (worker is stopped,
        // so the run stays Queued indefinitely).
        let scope = caller.turn_scope(thread_id.clone());
        let actor = caller.actor();
        let submitted_a = runtime
            .turn_coordinator
            .submit_turn(SubmitTurnRequest {
                scope: scope.clone(),
                actor: actor.clone(),
                accepted_message_ref: AcceptedMessageRef::new("msg:rejected-busy-a").unwrap(),
                source_binding_ref: SourceBindingRef::new("source:rejected-busy-a").unwrap(),
                reply_target_binding_ref: ReplyTargetBindingRef::new("reply:rejected-busy-a")
                    .unwrap(),
                requested_run_profile: None,
                idempotency_key: IdempotencyKey::new("rejected-busy-a").unwrap(),
                received_at: Utc::now(),
                requested_run_id: None,
                parent_run_id: None,
                subagent_depth: 0,
                spawn_tree_root_run_id: None,
                product_context: None,
            })
            .await
            .expect("message A submitted");
        let SubmitTurnResponse::Accepted {
            run_id: run_id_a, ..
        } = submitted_a;

        // Submit message B through the WebUI path — thread is busy, must get RejectedBusy.
        let response_b = bundle
            .api
            .submit_turn(
                caller.clone(),
                WebUiSendMessageRequest {
                    client_action_id: Some("send-rejected-busy-b".to_string()),
                    thread_id: Some(thread_id.to_string()),
                    content: Some("message B while thread is busy".to_string()),
                    attachments: Vec::new(),
                },
            )
            .await
            .expect("message B submit should not error");

        let RebornSubmitTurnResponse::RejectedBusy {
            notice: notice_b,
            active_run_id: busy_run_id,
            ..
        } = response_b
        else {
            panic!("expected RejectedBusy for message B, got {response_b:?}");
        };
        assert_eq!(
            busy_run_id,
            Some(run_id_a),
            "RejectedBusy should report run A as the active run"
        );
        assert!(
            !notice_b.is_empty(),
            "RejectedBusy response must carry a non-empty notice"
        );

        // Verify message B is stored with RejectedBusy status.
        let history = runtime
            .thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: runtime.thread_scope.clone(),
                thread_id: thread_id.clone(),
            })
            .await
            .expect("thread history after B");
        let rejected_messages: Vec<_> = history
            .messages
            .iter()
            .filter(|m| matches!(m.status, MessageStatus::RejectedBusy))
            .collect();
        assert_eq!(
            rejected_messages.len(),
            1,
            "exactly one message should be stored as RejectedBusy after thread-busy submit"
        );
        assert_eq!(
            rejected_messages[0].kind,
            MessageKind::User,
            "the RejectedBusy message must be of kind User"
        );

        // Cancel run A — this is the terminal event that (must NOT) auto-resubmit B.
        runtime
            .cancel_run(
                &scope,
                run_id_a,
                SanitizedCancelReason::UserRequested,
                "rejected-busy-cancel-a",
            )
            .await
            .expect("run A cancellation succeeds");

        // B must remain RejectedBusy — no auto-resubmission should have fired.
        let history_after_cancel = runtime
            .thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: runtime.thread_scope.clone(),
                thread_id: thread_id.clone(),
            })
            .await
            .expect("thread history after cancel");
        // Identify message B by the message_id we captured from the pre-cancel history.
        // Using the stable message_id (rather than a simple RejectedBusy count) ensures
        // a regression that leaves the RejectedBusy row AND adds a Submitted row for the
        // same message cannot slip past as "still one RejectedBusy".
        let msg_b_id = rejected_messages[0].message_id;

        let msg_b_after_cancel: Vec<_> = history_after_cancel
            .messages
            .iter()
            .filter(|m| m.message_id == msg_b_id)
            .collect();
        assert_eq!(
            msg_b_after_cancel.len(),
            1,
            "message B must appear exactly once in history after run A is cancelled"
        );
        assert_eq!(
            msg_b_after_cancel[0].status,
            MessageStatus::RejectedBusy,
            "message B must still be RejectedBusy after run A is cancelled — no auto-resubmission"
        );
        // Guard: no additional Submitted row must have been created for message B's message_id.
        let submitted_for_b: Vec<_> = history_after_cancel
            .messages
            .iter()
            .filter(|m| matches!(m.status, MessageStatus::Submitted) && m.message_id == msg_b_id)
            .collect();
        assert!(
            submitted_for_b.is_empty(),
            "no Submitted row must exist for message B after run A is cancelled — got {submitted_for_b:?}"
        );

        // Submit message C — thread is free again, must be Submitted.
        let response_c = bundle
            .api
            .submit_turn(
                caller.clone(),
                WebUiSendMessageRequest {
                    client_action_id: Some("send-rejected-busy-c".to_string()),
                    thread_id: Some(thread_id.to_string()),
                    content: Some("message C after thread is free".to_string()),
                    attachments: Vec::new(),
                },
            )
            .await
            .expect("message C submit should not error");

        assert!(
            matches!(response_c, RebornSubmitTurnResponse::Submitted { .. }),
            "message C must be accepted after run A is cancelled, got {response_c:?}"
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    struct MultiToolConfiguredCredentials;

    #[async_trait]
    impl crate::product_auth_runtime_credentials::RuntimeCredentialAccountSelectionService
        for MultiToolConfiguredCredentials
    {
        async fn select_configured_account_for_binding(
            &self,
            _lookup: ironclaw_auth::CredentialAccountSelectionRequest,
            _runtime_scope: ironclaw_auth::AuthProductScope,
        ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
            Err(ironclaw_auth::AuthProductError::CredentialMissing)
        }

        async fn select_unique_configured_runtime_account(
            &self,
            _request: crate::product_auth_runtime_credentials::RuntimeCredentialAccountSelectionRequest,
        ) -> Result<ironclaw_auth::CredentialAccount, ironclaw_auth::AuthProductError> {
            let now = chrono::Utc::now();
            Ok(ironclaw_auth::CredentialAccount {
                id: ironclaw_auth::CredentialAccountId::new(),
                scope: ironclaw_auth::AuthProductScope::new(
                    ironclaw_host_api::ResourceScope::local_default(
                        UserId::new("multi-tool-credential-user").expect("user id"),
                        ironclaw_host_api::InvocationId::new(),
                    )
                    .expect("resource scope"),
                    ironclaw_auth::AuthSurface::Api,
                ),
                provider: ironclaw_auth::AuthProviderId::new("test-provider").expect("provider id"),
                label: ironclaw_auth::CredentialAccountLabel::new("test-provider")
                    .expect("account label"),
                status: ironclaw_auth::CredentialAccountStatus::Configured,
                ownership: ironclaw_auth::CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(
                    ironclaw_host_api::SecretHandle::new("test-secret").expect("secret handle"),
                ),
                refresh_secret: None,
                scopes: Vec::new(),
                created_at: now,
                updated_at: now,
            })
        }
    }

    // ── Regression: scheduler liveness must not treat mutex contention as stopped ──

    /// Verify three invariants of the scheduler liveness check introduced to fix the
    /// `try_lock()` contention bug:
    ///
    /// 1. Before shutdown: liveness check says NOT stopped (atomic flag = false).
    /// 2. While mutex is momentarily held by another task: atomic flag is still false,
    ///    so the guard correctly treats that as "alive".
    /// 3. After graceful `shutdown()`: liveness check says stopped (atomic flag = true).
    ///
    /// The `stopped` atomic flag is the authoritative signal; `try_lock`
    /// failure now means "alive" rather than "stopped".
    #[tokio::test]
    async fn scheduler_liveness_not_stopped_under_contention() {
        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(RecordingGateway {
            reply: "liveness-test-reply".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });

        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("scheduler-liveness-owner", root.path().join("local-dev"))
                .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "scheduler-liveness-tenant".to_string(),
            agent_id: "scheduler-liveness-agent".to_string(),
            source_binding_id: "scheduler-liveness-source".to_string(),
            reply_target_binding_id: "scheduler-liveness-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: RUNTIME_SEND_TIMEOUT,
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input)
            .await
            .expect("runtime builds for liveness test");

        let conversation = runtime.new_conversation().await.expect("conversation");

        // Invariant 1: Before shutdown, the atomic stopped flag must be false.
        assert!(
            !runtime.turn_scheduler.atomic_stopped(),
            "scheduler_stopped must be false on a freshly built runtime"
        );

        // Invariant 2: While the scheduler handle mutex is held (simulating
        // shutdown/scheduler contention), the public submit path must NOT
        // return `WorkerStopped` — and must complete within a bounded timeout.
        //
        // `is_stopped()` uses `try_lock()` (non-blocking) on the handle mutex,
        // not `lock().await`, so holding the lock here cannot deadlock. Tokio's
        // Mutex is non-re-entrant: `try_lock()` inside `is_stopped()` will
        // fail (returning `Err`) because the current task already holds the guard.
        // The guard falls through to "alive" because the `stopped` flag is false.
        //
        // `notify()` sends through the notifier (not the handle mutex), so the
        // worker processes the turn while the test holds the handle. The
        // RecordingGateway resolves the model call synchronously, so the turn
        // reaches Completed. We assert the full Ok result to catch both the
        // liveness regression (WorkerStopped) and any other scheduler breakage.
        //
        // The surrounding `tokio::time::timeout` is the deadlock-regression
        // guard: if `is_stopped()` ever regresses from `try_lock()` to
        // `lock().await`, this test will panic with a clear message instead of
        // hanging CI indefinitely.
        {
            // Hold the tokio Mutex for the duration of the submit call.
            let _guard = runtime.turn_scheduler.handle_mutex().lock().await;

            let result = tokio::time::timeout(
                RUNTIME_SEND_TIMEOUT,
                runtime.send_user_message(&conversation, "liveness-probe"),
            )
            .await
            .expect(
                "send_user_message timed out while handle mutex was held — \
                 liveness guard likely regressed from try_lock() to lock().await, \
                 causing a deadlock",
            );

            assert!(
                result.is_ok(),
                "send_user_message must succeed (RecordingGateway completes the turn) \
                 while scheduler handle is merely contended (stopped=false); \
                 got: {result:?}"
            );
        } // guard released here — handle mutex is free again

        // Invariant 3: After the worker is stopped (flag = true), the public
        // submit path MUST return `WorkerStopped`.
        //
        // We use `stop_turn_runner_worker_for_manual_state_test` instead of
        // `shutdown()` because `shutdown()` consumes `self`, which would prevent
        // us from calling `send_user_message` afterward to exercise the guard.
        stop_turn_runner_worker_for_manual_state_test(&runtime).await;

        assert!(
            runtime.turn_scheduler.atomic_stopped(),
            "scheduler_stopped must be true after stop helper"
        );

        let result_after_stop = runtime
            .send_user_message(&conversation, "post-stop-probe")
            .await;
        assert!(
            matches!(
                result_after_stop,
                Err(super::RebornRuntimeError::WorkerStopped)
            ),
            "send_user_message must return WorkerStopped after scheduler is stopped; \
             got: {result_after_stop:?}"
        );

        // shutdown() handles the already-taken scheduler handle gracefully.
        runtime.shutdown().await.expect("runtime shutdown");
    }

    /// Companion test: `stop_turn_runner_worker_for_manual_state_test` (the test-only
    /// helper used by many existing tests) must also set `scheduler_stopped = true`
    /// so the liveness guard correctly reports stopped after it is called.
    #[tokio::test]
    async fn scheduler_liveness_stopped_after_test_helper_stops_worker() {
        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(RecordingGateway {
            reply: "liveness-helper-test-reply".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });

        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(
                "scheduler-liveness-helper-owner",
                root.path().join("local-dev"),
            )
            .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "scheduler-liveness-helper-tenant".to_string(),
            agent_id: "scheduler-liveness-helper-agent".to_string(),
            source_binding_id: "scheduler-liveness-helper-source".to_string(),
            reply_target_binding_id: "scheduler-liveness-helper-reply".to_string(),
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input)
            .await
            .expect("runtime builds for helper-stopped test");

        // Before stopping: not stopped.
        assert!(
            !runtime.turn_scheduler.atomic_stopped(),
            "scheduler_stopped must be false before stop helper runs"
        );

        stop_turn_runner_worker_for_manual_state_test(&runtime).await;

        // After the test helper stops the worker: flag must be true.
        assert!(
            runtime.turn_scheduler.atomic_stopped(),
            "scheduler_stopped must be true after stop_turn_runner_worker_for_manual_state_test"
        );

        // shutdown() handles the already-taken scheduler handle gracefully
        // via the `if let Some` guard — safe to call after the test helper.
        runtime.shutdown().await.expect("runtime shutdown");
    }

    /// After `stop_turn_runner_worker_for_manual_state_test` sets
    /// `scheduler_stopped = true`, `send_user_message` must immediately return
    /// `Err(RebornRuntimeError::WorkerStopped)` without submitting the turn.
    #[tokio::test]
    async fn scheduler_stopped_rejects_send_user_message() {
        let root = tempfile::tempdir().expect("tempdir");
        let gateway = Arc::new(RecordingGateway {
            reply: "stopped-reject-reply".to_string(),
            requests: Arc::new(StdMutex::new(Vec::new())),
        });

        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev(
                "scheduler-stopped-reject-owner",
                root.path().join("local-dev"),
            )
            .with_runtime_policy(local_dev_runtime_policy()),
        )
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "scheduler-stopped-reject-tenant".to_string(),
            agent_id: "scheduler-stopped-reject-agent".to_string(),
            source_binding_id: "scheduler-stopped-reject-source".to_string(),
            reply_target_binding_id: "scheduler-stopped-reject-reply".to_string(),
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input)
            .await
            .expect("runtime builds for stopped-reject test");

        let conversation = runtime.new_conversation().await.expect("conversation");

        // Capture thread history before the stopped-send to verify no side effects.
        let thread_service = runtime.session_thread_service();
        let thread_scope = runtime.thread_scope.clone();
        let history_before = thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: thread_scope.clone(),
                thread_id: conversation.0.clone(),
            })
            .await
            .expect("list history before stopped send");

        stop_turn_runner_worker_for_manual_state_test(&runtime).await;

        let result = runtime.send_user_message(&conversation, "hi").await;
        assert!(
            matches!(result, Err(RebornRuntimeError::WorkerStopped)),
            "send_user_message must return WorkerStopped when scheduler is stopped, got: {result:?}"
        );

        // Assert no side effects: history must not grow after the rejected send.
        let history_after = thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: thread_scope,
                thread_id: conversation.0.clone(),
            })
            .await
            .expect("list history after stopped send");
        assert_eq!(
            history_before.messages.len(),
            history_after.messages.len(),
            "send_user_message must not write any messages when WorkerStopped is returned"
        );

        // shutdown() handles the already-taken scheduler handle gracefully.
        runtime.shutdown().await.expect("runtime shutdown");
    }
}
