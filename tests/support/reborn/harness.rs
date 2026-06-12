//! Reborn binary-E2E harness.
//!
//! This harness drives the product caller path used by the #3702 validation
//! ports:
//!
//! inbound bytes -> ProductAdapter -> DefaultProductWorkflow ->
//! DefaultInboundTurnService -> DefaultTurnCoordinator -> TurnRunnerWorker ->
//! Reborn planned agent loop -> model/capability/transcript evidence.
//!
//! Documented test-support substitutions:
//! - the model gateway is scripted trace replay;
//! - the capability port is a local recording echo/approval port;
//! - external internet, delivery, and OAuth are not exercised by this harness.

#![allow(dead_code)] // Shared by staged Reborn binary-E2E validation ports.

use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use ironclaw_approvals::{ApprovalResolver, LeaseApproval};
use ironclaw_authorization::{GrantAuthorizer, TrustAwareCapabilityDispatchAuthorizer};
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::{
    BackendCapabilities, BackendId, BackendKind, CompositeRootFilesystem, ContentKind,
    InMemoryBackend, IndexPolicy, LocalFilesystem, MountDescriptor, RootFilesystem,
    ScopedFilesystem, StorageClass,
};
use ironclaw_host_api::{
    Action, AgentId, ApprovalRequestId, CapabilityDescriptor, CapabilityGrant, CapabilityGrantId,
    CapabilityId, CapabilitySet, CredentialStageError, Decision, EffectKind, ExecutionContext,
    ExtensionId, GrantConstraints, HostPath, MountAlias, MountGrant, MountPermissions, MountView,
    NetworkPolicy, NetworkScheme, NetworkTargetPattern, Obligation, Obligations, PackageId,
    Principal, ProjectId, ResourceEstimate, ResourceScope, RuntimeCredentialAccountProviderId,
    RuntimeHttpEgress, RuntimeHttpEgressError, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse,
    RuntimeKind, SecretHandle, TenantId, ThreadId, TrustClass, UserId, VirtualPath,
};
use ironclaw_host_runtime::{
    APPLY_PATCH_CAPABILITY_ID, BUILTIN_FIRST_PARTY_PROVIDER, CancelRuntimeWorkOutcome,
    CancelRuntimeWorkRequest, CapabilitySurfacePolicy,
    CapabilitySurfaceVersion as HostRuntimeCapabilitySurfaceVersion, ECHO_CAPABILITY_ID,
    GLOB_CAPABILITY_ID, GREP_CAPABILITY_ID, HTTP_CAPABILITY_ID, HTTP_SAVE_CAPABILITY_ID,
    HostRuntime, HostRuntimeError, HostRuntimeHealth, HostRuntimeServices, HostRuntimeStatus,
    JSON_CAPABILITY_ID, LIST_DIR_CAPABILITY_ID, MEMORY_READ_CAPABILITY_ID,
    MEMORY_SEARCH_CAPABILITY_ID, MEMORY_TREE_CAPABILITY_ID, MEMORY_WRITE_CAPABILITY_ID,
    READ_FILE_CAPABILITY_ID, RuntimeCapabilityOutcome, RuntimeCapabilityRequest,
    RuntimeCapabilityResumeRequest, RuntimeCredentialAccessSecret, RuntimeCredentialAccountRequest,
    RuntimeCredentialAccountResolver, RuntimeStatusRequest, SHELL_CAPABILITY_ID,
    SKILL_INSTALL_CAPABILITY_ID, SKILL_LIST_CAPABILITY_ID, SKILL_REMOVE_CAPABILITY_ID,
    SPAWN_SUBAGENT_CAPABILITY_ID, SurfaceKind, TIME_CAPABILITY_ID, TRIGGER_CREATE_CAPABILITY_ID,
    TRIGGER_LIST_CAPABILITY_ID, TRIGGER_REMOVE_CAPABILITY_ID,
    VisibleCapabilityRequest as RuntimeVisibleCapabilityRequest,
    VisibleCapabilitySurface as RuntimeVisibleCapabilitySurface, WRITE_FILE_CAPABILITY_ID,
    builtin_first_party_handlers, builtin_first_party_package,
};
use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilityResolveError, CapabilityResultWrite,
    CapabilitySurfaceProfileResolver, DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID,
    HostIdentityContextBuildError, HostIdentityContextCandidate, HostIdentityContextSource,
    HostManagedModelRequest, HostRuntimeLoopCapabilityPortFactory, JsonSpawnSubagentInputCodec,
    LoopCapabilityPortFactory, LoopCapabilityResultWriter,
};
use ironclaw_network::{
    NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse, NetworkUsage,
    PolicyNetworkHttpEgress, ReqwestNetworkTransport,
};
use ironclaw_product_adapters::{
    ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload, ProductTriggerReason,
    ProductWorkflow,
};
use ironclaw_product_workflow::{
    ConversationBindingService, DefaultInboundTurnService, DefaultProductWorkflow,
    IdempotencyLedger, InboundTurnService, ProductConversationRouteKind, ResolveBindingRequest,
    ResolvedBinding,
};
use ironclaw_reborn::subagent::{
    flavors::StaticSubagentDefinitionResolver, gate_resolution::BoundedSubagentGateResolutionStore,
    goal_store::InMemoryBoundedSubagentGoalStore,
};
use ironclaw_reborn::{
    loop_exit_applier::{
        BlockedEvidenceRequest, CompletionEvidenceRequest, FailureEvidenceRequest,
        FinalCheckpointEvidenceRequest, LoopExitEvidencePort, ThreadCheckpointLoopExitEvidencePort,
    },
    runtime::{
        DefaultPlannedRuntimeConfig, DefaultPlannedRuntimeParts, RebornRuntimeLoopComposition,
        RuntimeTurnStateStore, build_default_planned_runtime,
    },
    turn_runner::{TurnRunnerWakeSender, TurnRunnerWorker, TurnRunnerWorkerConfig},
};
use ironclaw_reborn_composition::{
    ProductLiveCapabilityIo, ProductLiveVisibleCapabilityRequestConfig, RebornBuildInput,
    RebornLocalDevApprovalTestParts, build_reborn_services, visible_capability_request_for_run,
};
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_secrets::{
    InMemorySecretStore, SecretLease, SecretLeaseId, SecretLeaseStatus, SecretMaterial,
    SecretMetadata, SecretStore, SecretStoreError,
};
use ironclaw_threads::{
    FilesystemSessionThreadService, SessionThreadService, ThreadHistoryRequest,
    ThreadMessageRecord, ThreadScope,
};
use ironclaw_trust::{AdminConfig, AdminEntry, HostTrustAssignment, HostTrustPolicy};
use ironclaw_trust::{EffectiveTrustClass, TrustDecision};
use ironclaw_turns::{
    CancelRunRequest, FilesystemTurnStateStore, GateRef, GetLoopCheckpointRequest,
    GetRunStateRequest, IdempotencyKey, InMemoryCheckpointStateStore, LoopBlockedKind,
    LoopCheckpointKind, LoopCheckpointStore, LoopGateRef, LoopResultRef, ReplyTargetBindingRef,
    ResumeTurnRequest, SanitizedCancelReason, SourceBindingRef, TurnActor, TurnCoordinator,
    TurnError, TurnRunId, TurnRunRecord, TurnRunState, TurnScope, TurnSpawnTreeStateStore,
    TurnStateStore, TurnStatus,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityBatchInvocation,
        CapabilityBatchOutcome, CapabilityCallCandidate, CapabilityDescriptorView,
        CapabilityInputRef, CapabilityInvocation, CapabilityOutcome, CapabilityResultMessage,
        CapabilitySurfaceVersion, ConcurrencyHint, LoopCapabilityPort, LoopHostMilestone,
        LoopHostMilestoneKind, LoopHostMilestoneSink, LoopRunContext, ParentLoopOutput, PromptMode,
        ProviderToolCall, ProviderToolCallReplay, ProviderToolDefinition, VisibleCapabilityRequest,
        VisibleCapabilitySurface,
    },
};
use ironclaw_wasm::{WitToolHost, WitToolRuntimeConfig};
use serde_json::json;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::{
    config::WaitConfig,
    filesystem::local_filesystem,
    github as github_support,
    model_replay::RebornTraceReplayModelGateway,
    product_workflow::{RebornProductWorkflowHarness, resource_scope},
    session_thread::RebornThreadHarness,
    test_adapter::{RebornTestIngress, RebornTestProductAdapter},
};

pub type HarnessWaitConfig = WaitConfig;

const TEST_CAPABILITY_ID: &str = "test.echo";
const TEST_CAPABILITY_SURFACE_VERSION: &str = "trace_replay_v1";
const SUBAGENT_ALLOWED_TEST_TOOL_NAME: &str = "test_read_file";

type HarnessResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
type HarnessCapabilityParts = (
    Arc<dyn LoopCapabilityPortFactory>,
    Arc<dyn CapabilitySurfaceProfileResolver>,
    Arc<dyn ironclaw_loop_support::LoopCapabilityInputResolver>,
    Arc<dyn LoopCapabilityResultWriter>,
    HarnessCapabilityRecorder,
);

pub struct RebornBinaryE2EHarness {
    ingress: RebornTestIngress,
    workflow: DefaultProductWorkflow,
    external_conversation_id: String,
    binding: ResolvedBinding,
    thread_scope: ThreadScope,
    turn_scope: TurnScope,
    turn_store: Arc<FilesystemTurnStateStore<LocalFilesystem>>,
    coordinator: Arc<dyn TurnCoordinator>,
    _product_harness: RebornProductWorkflowHarness,
    thread_harness: RebornThreadHarness,
    model_gateway: RebornTraceReplayModelGateway,
    capability_recorder: HarnessCapabilityRecorder,
    milestone_sink: Arc<ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink>,
    worker: Arc<TurnRunnerWorker>,
    cancel: CancellationToken,
    worker_tasks: Vec<JoinHandle<()>>,
    _turn_root: Arc<tempfile::TempDir>,
    _wake_sender: TurnRunnerWakeSender,
}

pub struct SubmittedTurn {
    pub ack: ProductInboundAck,
    pub run_id: TurnRunId,
    pub thread_id: ThreadId,
    pub thread_scope: ThreadScope,
    pub scope: TurnScope,
    pub actor: TurnActor,
}

#[derive(Clone)]
pub struct RebornHarnessSharedStorage {
    product_backend: Arc<LocalFilesystem>,
    product_root: Arc<tempfile::TempDir>,
    thread_backend: Arc<LocalFilesystem>,
    thread_root: Arc<tempfile::TempDir>,
    turn_backend: Arc<LocalFilesystem>,
    turn_root: Arc<tempfile::TempDir>,
}

impl RebornHarnessSharedStorage {
    pub fn new() -> HarnessResult<Self> {
        let product_root = Arc::new(tempfile::tempdir()?);
        let thread_root = Arc::new(tempfile::tempdir()?);
        let turn_root = Arc::new(tempfile::tempdir()?);
        Ok(Self {
            product_backend: Arc::new(local_filesystem(product_root.path())?),
            product_root,
            thread_backend: Arc::new(local_filesystem(thread_root.path())?),
            thread_root,
            turn_backend: Arc::new(local_filesystem(turn_root.path())?),
            turn_root,
        })
    }
}

#[derive(Debug, Clone)]
pub struct RecordedCapabilityResult {
    pub capability_id: CapabilityId,
    pub output: serde_json::Value,
}

enum HarnessCapabilityMode {
    Recording(RecordingTestCapabilityPort),
    HostRuntime(Arc<HostRuntimeCapabilityHarness>),
}

#[derive(Clone)]
enum HarnessCapabilityRecorder {
    Recording(Arc<RecordingTestCapabilityPort>),
    HostRuntime(Arc<HostRuntimeCapabilityHarness>),
}

impl HarnessCapabilityRecorder {
    fn invocations(&self) -> Vec<CapabilityInvocation> {
        match self {
            Self::Recording(port) => port.invocations(),
            Self::HostRuntime(harness) => harness.invocations(),
        }
    }

    fn workspace_file_path(&self, relative: &str) -> Option<PathBuf> {
        match self {
            Self::Recording(_) => None,
            Self::HostRuntime(harness) => Some(harness.workspace_file_path(relative)),
        }
    }

    fn capability_results(&self) -> Vec<RecordedCapabilityResult> {
        match self {
            Self::Recording(_) => Vec::new(),
            Self::HostRuntime(harness) => harness.capability_results(),
        }
    }

    fn runtime_http_requests(&self) -> Vec<RuntimeHttpEgressRequest> {
        match self {
            Self::Recording(_) => Vec::new(),
            Self::HostRuntime(harness) => harness.runtime_http_requests(),
        }
    }

    fn network_http_requests(&self) -> Vec<NetworkHttpRequest> {
        match self {
            Self::Recording(_) => Vec::new(),
            Self::HostRuntime(harness) => harness.network_http_requests(),
        }
    }

    async fn approve_local_dev_gate(&self, gate_ref: &GateRef) -> HarnessResult<()> {
        match self {
            Self::Recording(_) => {
                Err("recording capability port has no local-dev approvals".into())
            }
            Self::HostRuntime(harness) => harness.approve_local_dev_gate(gate_ref).await,
        }
    }
}

impl RebornBinaryE2EHarness {
    pub async fn reply_only(
        conversation_id: &str,
        reply: impl Into<String>,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway(
            conversation_id,
            RebornTraceReplayModelGateway::with_responses([
                ironclaw_loop_support::HostManagedModelResponse::assistant_reply(reply),
            ]),
            RecordingTestCapabilityPort::echo(),
        )
        .await
    }

    pub async fn with_model_gateway(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_port: RecordingTestCapabilityPort,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway_options(conversation_id, model_gateway, capability_port, false)
            .await
    }

    pub async fn with_model_gateway_unscoped_worker(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_port: RecordingTestCapabilityPort,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway_options_and_worker_scope(
            conversation_id,
            model_gateway,
            capability_port,
            false,
            false,
        )
        .await
    }

    pub async fn with_harness_blocked_evidence_unscoped_worker(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_port: RecordingTestCapabilityPort,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway_options_and_worker_scope(
            conversation_id,
            model_gateway,
            capability_port,
            true,
            false,
        )
        .await
    }

    pub async fn with_model_gateway_scope_shared_storage_unscoped_worker(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_port: RecordingTestCapabilityPort,
        scope: ResourceScope,
        shared_storage: RebornHarnessSharedStorage,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway_scope_identity_source_trigger_installation_shared_storage_unscoped_worker(
            conversation_id,
            model_gateway,
            capability_port,
            scope,
            Arc::new(EmptyIdentityContextSource),
            ProductTriggerReason::DirectChat,
            "reborn-test",
            "install-1",
            "alice",
            shared_storage,
        )
        .await
    }

    pub async fn with_model_gateway_scope_installation_shared_storage_unscoped_worker(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_port: RecordingTestCapabilityPort,
        scope: ResourceScope,
        adapter_id: &str,
        installation_id: &str,
        shared_storage: RebornHarnessSharedStorage,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway_scope_initial_actor_installation_shared_storage_unscoped_worker(
            conversation_id,
            "alice",
            model_gateway,
            capability_port,
            scope,
            adapter_id,
            installation_id,
            shared_storage,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn with_model_gateway_scope_initial_actor_installation_shared_storage_unscoped_worker(
        conversation_id: &str,
        initial_actor_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_port: RecordingTestCapabilityPort,
        scope: ResourceScope,
        adapter_id: &str,
        installation_id: &str,
        shared_storage: RebornHarnessSharedStorage,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway_scope_identity_source_trigger_installation_shared_storage_unscoped_worker(
            conversation_id,
            model_gateway,
            capability_port,
            scope,
            Arc::new(EmptyIdentityContextSource),
            ProductTriggerReason::DirectChat,
            adapter_id,
            installation_id,
            initial_actor_id,
            shared_storage,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn with_model_gateway_scope_identity_source_trigger_installation_shared_storage_unscoped_worker(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_port: RecordingTestCapabilityPort,
        scope: ResourceScope,
        identity_context_source: Arc<dyn HostIdentityContextSource>,
        initial_trigger: ProductTriggerReason,
        adapter_id: &str,
        installation_id: &str,
        initial_actor_id: &str,
        shared_storage: RebornHarnessSharedStorage,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway_capability_mode_identity_source_trigger_worker_scope_storage_and_adapter(
            conversation_id,
            model_gateway,
            HarnessCapabilityMode::Recording(capability_port),
            false,
            false,
            initial_trigger,
            identity_context_source,
            scope,
            Some(shared_storage),
            adapter_id,
            installation_id,
            initial_actor_id,
        )
        .await
    }

    pub async fn with_model_gateway_identity_source_unscoped_worker(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_port: RecordingTestCapabilityPort,
        identity_context_source: Arc<dyn HostIdentityContextSource>,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway_options_identity_source_trigger_and_worker_scope(
            conversation_id,
            model_gateway,
            capability_port,
            false,
            false,
            ProductTriggerReason::DirectChat,
            identity_context_source,
        )
        .await
    }

    pub async fn with_model_gateway_identity_source_unscoped_shared_worker(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_port: RecordingTestCapabilityPort,
        identity_context_source: Arc<dyn HostIdentityContextSource>,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway_options_identity_source_trigger_and_worker_scope(
            conversation_id,
            model_gateway,
            capability_port,
            false,
            false,
            ProductTriggerReason::BotMention,
            identity_context_source,
        )
        .await
    }

    pub async fn with_host_runtime_file_capabilities(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
    ) -> HarnessResult<Self> {
        let host_runtime = Arc::new(HostRuntimeCapabilityHarness::file_tools().await?);
        Self::with_model_gateway_capability_mode(
            conversation_id,
            model_gateway,
            HarnessCapabilityMode::HostRuntime(host_runtime),
            true,
        )
        .await
    }

    pub async fn with_host_runtime_file_capabilities_requiring_approval(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
    ) -> HarnessResult<Self> {
        let host_runtime =
            Arc::new(HostRuntimeCapabilityHarness::file_tools_requiring_approval().await?);
        Self::with_model_gateway_capability_mode(
            conversation_id,
            model_gateway,
            HarnessCapabilityMode::HostRuntime(host_runtime),
            true,
        )
        .await
    }

    pub async fn with_host_runtime_write_only(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
    ) -> HarnessResult<Self> {
        let host_runtime = Arc::new(HostRuntimeCapabilityHarness::write_only().await?);
        Self::with_model_gateway_capability_mode(
            conversation_id,
            model_gateway,
            HarnessCapabilityMode::HostRuntime(host_runtime),
            false,
        )
        .await
    }

    pub async fn with_host_runtime_coding_read_capabilities(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
    ) -> HarnessResult<Self> {
        let host_runtime = Arc::new(HostRuntimeCapabilityHarness::coding_read_tools().await?);
        Self::with_model_gateway_capability_mode(
            conversation_id,
            model_gateway,
            HarnessCapabilityMode::HostRuntime(host_runtime),
            false,
        )
        .await
    }

    pub async fn with_host_runtime_core_builtin_capabilities(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
    ) -> HarnessResult<Self> {
        let host_runtime = Arc::new(HostRuntimeCapabilityHarness::core_builtin_tools().await?);
        Self::with_model_gateway_capability_mode(
            conversation_id,
            model_gateway,
            HarnessCapabilityMode::HostRuntime(host_runtime),
            false,
        )
        .await
    }

    pub async fn with_host_runtime_process_capabilities(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
    ) -> HarnessResult<Self> {
        let host_runtime = Arc::new(HostRuntimeCapabilityHarness::process_tools().await?);
        Self::with_model_gateway_capability_mode(
            conversation_id,
            model_gateway,
            HarnessCapabilityMode::HostRuntime(host_runtime),
            false,
        )
        .await
    }

    pub async fn with_host_runtime_skill_management_capabilities(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
    ) -> HarnessResult<Self> {
        let host_runtime = Arc::new(HostRuntimeCapabilityHarness::skill_management_tools().await?);
        Self::with_model_gateway_capability_mode(
            conversation_id,
            model_gateway,
            HarnessCapabilityMode::HostRuntime(host_runtime),
            false,
        )
        .await
    }

    pub async fn with_host_runtime_trigger_management_capabilities(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
    ) -> HarnessResult<Self> {
        let host_runtime =
            Arc::new(HostRuntimeCapabilityHarness::trigger_management_tools().await?);
        Self::with_model_gateway_capability_mode(
            conversation_id,
            model_gateway,
            HarnessCapabilityMode::HostRuntime(host_runtime),
            false,
        )
        .await
    }

    pub async fn with_host_runtime_core_builtin_capabilities_network_policy(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        network_policy: NetworkPolicy,
    ) -> HarnessResult<Self> {
        let host_runtime = Arc::new(
            HostRuntimeCapabilityHarness::core_builtin_tools_with_network_policy(network_policy)
                .await?,
        );
        Self::with_model_gateway_capability_mode(
            conversation_id,
            model_gateway,
            HarnessCapabilityMode::HostRuntime(host_runtime),
            false,
        )
        .await
    }

    pub async fn with_host_runtime_core_builtin_capabilities_live_http_egress(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        network_policy: NetworkPolicy,
    ) -> HarnessResult<Self> {
        let host_runtime = Arc::new(
            HostRuntimeCapabilityHarness::core_builtin_tools_with_live_http_egress(network_policy)
                .await?,
        );
        Self::with_model_gateway_capability_mode(
            conversation_id,
            model_gateway,
            HarnessCapabilityMode::HostRuntime(host_runtime),
            false,
        )
        .await
    }

    pub async fn with_host_runtime_github_issue_capabilities(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
    ) -> HarnessResult<Self> {
        let host_runtime = Arc::new(HostRuntimeCapabilityHarness::github_issue_tools().await?);
        Self::with_model_gateway_capability_mode(
            conversation_id,
            model_gateway,
            HarnessCapabilityMode::HostRuntime(host_runtime),
            false,
        )
        .await
    }

    pub async fn with_harness_blocked_evidence(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_port: RecordingTestCapabilityPort,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway_options(conversation_id, model_gateway, capability_port, true)
            .await
    }

    async fn with_model_gateway_options(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_port: RecordingTestCapabilityPort,
        accept_harness_blocked_evidence: bool,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway_options_and_worker_scope(
            conversation_id,
            model_gateway,
            capability_port,
            accept_harness_blocked_evidence,
            true,
        )
        .await
    }

    async fn with_model_gateway_options_and_worker_scope(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_port: RecordingTestCapabilityPort,
        accept_harness_blocked_evidence: bool,
        restrict_worker_to_initial_scope: bool,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway_options_identity_source_and_worker_scope(
            conversation_id,
            model_gateway,
            capability_port,
            accept_harness_blocked_evidence,
            restrict_worker_to_initial_scope,
            Arc::new(EmptyIdentityContextSource),
        )
        .await
    }

    async fn with_model_gateway_options_identity_source_and_worker_scope(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_port: RecordingTestCapabilityPort,
        accept_harness_blocked_evidence: bool,
        restrict_worker_to_initial_scope: bool,
        identity_context_source: Arc<dyn HostIdentityContextSource>,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway_options_identity_source_trigger_and_worker_scope(
            conversation_id,
            model_gateway,
            capability_port,
            accept_harness_blocked_evidence,
            restrict_worker_to_initial_scope,
            ProductTriggerReason::DirectChat,
            identity_context_source,
        )
        .await
    }

    async fn with_model_gateway_options_identity_source_trigger_and_worker_scope(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_port: RecordingTestCapabilityPort,
        accept_harness_blocked_evidence: bool,
        restrict_worker_to_initial_scope: bool,
        initial_trigger: ProductTriggerReason,
        identity_context_source: Arc<dyn HostIdentityContextSource>,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway_capability_mode_identity_source_trigger_and_worker_scope(
            conversation_id,
            model_gateway,
            HarnessCapabilityMode::Recording(capability_port),
            accept_harness_blocked_evidence,
            restrict_worker_to_initial_scope,
            initial_trigger,
            identity_context_source,
        )
        .await
    }

    async fn with_model_gateway_capability_mode(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_mode: HarnessCapabilityMode,
        accept_harness_blocked_evidence: bool,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway_capability_mode_and_worker_scope(
            conversation_id,
            model_gateway,
            capability_mode,
            accept_harness_blocked_evidence,
            true,
        )
        .await
    }

    async fn with_model_gateway_capability_mode_and_worker_scope(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_mode: HarnessCapabilityMode,
        accept_harness_blocked_evidence: bool,
        restrict_worker_to_initial_scope: bool,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway_capability_mode_identity_source_and_worker_scope(
            conversation_id,
            model_gateway,
            capability_mode,
            accept_harness_blocked_evidence,
            restrict_worker_to_initial_scope,
            Arc::new(EmptyIdentityContextSource),
        )
        .await
    }

    async fn with_model_gateway_capability_mode_identity_source_and_worker_scope(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_mode: HarnessCapabilityMode,
        accept_harness_blocked_evidence: bool,
        restrict_worker_to_initial_scope: bool,
        identity_context_source: Arc<dyn HostIdentityContextSource>,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway_capability_mode_identity_source_trigger_and_worker_scope(
            conversation_id,
            model_gateway,
            capability_mode,
            accept_harness_blocked_evidence,
            restrict_worker_to_initial_scope,
            ProductTriggerReason::DirectChat,
            identity_context_source,
        )
        .await
    }

    async fn with_model_gateway_capability_mode_identity_source_trigger_and_worker_scope(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_mode: HarnessCapabilityMode,
        accept_harness_blocked_evidence: bool,
        restrict_worker_to_initial_scope: bool,
        initial_trigger: ProductTriggerReason,
        identity_context_source: Arc<dyn HostIdentityContextSource>,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway_capability_mode_identity_source_trigger_worker_scope_storage_and_adapter(
            conversation_id,
            model_gateway,
            capability_mode,
            accept_harness_blocked_evidence,
            restrict_worker_to_initial_scope,
            initial_trigger,
            identity_context_source,
            product_scope(),
            None,
            "reborn-test",
            "install-1",
            "alice",
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn with_model_gateway_capability_mode_identity_source_trigger_worker_scope_storage_and_adapter(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_mode: HarnessCapabilityMode,
        accept_harness_blocked_evidence: bool,
        restrict_worker_to_initial_scope: bool,
        initial_trigger: ProductTriggerReason,
        identity_context_source: Arc<dyn HostIdentityContextSource>,
        product_scope: ResourceScope,
        shared_storage: Option<RebornHarnessSharedStorage>,
        adapter_id: &str,
        installation_id: &str,
        initial_actor_id: &str,
    ) -> HarnessResult<Self> {
        let adapter = RebornTestProductAdapter::new(adapter_id, installation_id)?;
        let ingress = RebornTestIngress::new(adapter);
        let product_harness = if let Some(storage) = shared_storage.as_ref() {
            RebornProductWorkflowHarness::filesystem_shared_backend(
                product_scope.clone(),
                Arc::clone(&storage.product_backend),
                Arc::clone(&storage.product_root),
            )?
        } else {
            RebornProductWorkflowHarness::filesystem_temp(product_scope)?
        };
        let binding = product_harness
            .binding_service()?
            .resolve_binding(binding_request_with_trigger_and_actor(
                &ingress,
                conversation_id,
                initial_actor_id,
                initial_trigger,
            )?)
            .await?;
        let thread_scope = thread_scope_from_binding_with_route_kind(
            &binding,
            route_kind_for_trigger(initial_trigger),
        )?;
        let turn_scope = TurnScope::new_with_owner(
            binding.tenant_id.clone(),
            binding.agent_id.clone(),
            binding.project_id.clone(),
            binding.thread_id.clone(),
            binding.subject_user_id.clone(),
        );
        let thread_harness = if let Some(storage) = shared_storage.as_ref() {
            RebornThreadHarness::filesystem_shared_backend(
                thread_scope.clone(),
                Arc::clone(&storage.thread_backend),
                Arc::clone(&storage.thread_root),
            )?
        } else {
            RebornThreadHarness::filesystem_temp(thread_scope.clone())?
        };
        let (turn_backend, turn_root) = if let Some(storage) = shared_storage.as_ref() {
            (
                Arc::clone(&storage.turn_backend),
                Arc::clone(&storage.turn_root),
            )
        } else {
            let turn_root = Arc::new(tempfile::tempdir()?);
            (Arc::new(local_filesystem(turn_root.path())?), turn_root)
        };
        let turn_store = Arc::new(FilesystemTurnStateStore::new(scoped_turns_fs(
            turn_backend,
            &binding,
        )?));
        let checkpoint_state_store = Arc::new(InMemoryCheckpointStateStore::default());
        let loop_checkpoint_store: Arc<dyn LoopCheckpointStore> = turn_store.clone();
        let milestone_sink =
            Arc::new(ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink::default());
        let (
            capability_factory,
            capability_surface_resolver,
            capability_input_resolver,
            capability_result_writer,
            capability_recorder,
        ) = capability_mode.into_parts(milestone_sink.clone())?;
        let turn_state_for_evidence: Arc<dyn TurnStateStore> = turn_store.clone();
        let evidence = Arc::new(HarnessLoopExitEvidencePort {
            inner: ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
                thread_harness.service.clone(),
                turn_state_for_evidence,
                Arc::clone(&loop_checkpoint_store),
                thread_scope.clone(),
            ),
            loop_checkpoint_store: Arc::clone(&loop_checkpoint_store),
            accept_harness_blocked_evidence,
        });
        let turn_state_for_runtime: Arc<dyn RuntimeTurnStateStore> = turn_store.clone();
        let composition = build_default_planned_runtime(DefaultPlannedRuntimeParts {
            turn_state: turn_state_for_runtime,
            thread_service: thread_harness.service.clone()
                as Arc<dyn ironclaw_threads::SessionThreadService>,
            thread_scope: thread_scope.clone(),
            model_gateway: Arc::new(model_gateway.clone()),
            checkpoint_state_store,
            loop_checkpoint_store,
            milestone_sink: milestone_sink.clone(),
            capability_factory,
            capability_surface_resolver,
            capability_result_writer,
            subagent_goal_store: Arc::new(InMemoryBoundedSubagentGoalStore::new()),
            subagent_gate_store: Arc::new(BoundedSubagentGateResolutionStore::new()),
            subagent_definition_resolver: Arc::new(StaticSubagentDefinitionResolver),
            subagent_spawn_input_codec: Arc::new(JsonSpawnSubagentInputCodec::new(
                capability_input_resolver,
            )),
            subagent_spawn_limits: ironclaw_loop_support::SubagentSpawnLimits::default(),
            loop_exit_evidence: evidence,
            config: DefaultPlannedRuntimeConfig {
                worker: TurnRunnerWorkerConfig {
                    heartbeat_interval: Duration::from_millis(20),
                    poll_interval: Duration::from_millis(10),
                    scope_filter: restrict_worker_to_initial_scope.then(|| turn_scope.clone()),
                },
                ..DefaultPlannedRuntimeConfig::default()
            },
            model_route_resolver: None,
            cancellation_factory: None,
            skill_context_source: None,
            input_queue: None,
            identity_context_source,
            model_policy_guard: None,
            model_budget_accountant: None,
            safety_context: None,
            hook_dispatcher_builder_factory: None,
            hook_security_audit_sink: None,
            turn_event_sink: None,
        })?;
        let binding_service: Arc<dyn ConversationBindingService> =
            Arc::new(product_harness.binding_service()?);
        let inbound: Arc<dyn InboundTurnService> = Arc::new(DefaultInboundTurnService::new(
            Arc::clone(&binding_service),
            thread_harness.service_instance()?,
            composition.coordinator.clone(),
        ));
        let ledger: Arc<dyn IdempotencyLedger> = Arc::new(product_harness.idempotency_ledger());
        let workflow = DefaultProductWorkflow::new(inbound, ledger, binding_service);

        Ok(Self::from_composition(
            ingress,
            workflow,
            conversation_id.to_string(),
            binding,
            thread_scope,
            turn_scope,
            turn_store,
            product_harness,
            thread_harness,
            model_gateway,
            capability_recorder,
            milestone_sink,
            composition,
            turn_root,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn from_composition(
        ingress: RebornTestIngress,
        workflow: DefaultProductWorkflow,
        external_conversation_id: String,
        binding: ResolvedBinding,
        thread_scope: ThreadScope,
        turn_scope: TurnScope,
        turn_store: Arc<FilesystemTurnStateStore<LocalFilesystem>>,
        product_harness: RebornProductWorkflowHarness,
        thread_harness: RebornThreadHarness,
        model_gateway: RebornTraceReplayModelGateway,
        capability_recorder: HarnessCapabilityRecorder,
        milestone_sink: Arc<ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink>,
        composition: RebornRuntimeLoopComposition<
            dyn SessionThreadService,
            RebornTraceReplayModelGateway,
        >,
        turn_root: Arc<tempfile::TempDir>,
    ) -> Self {
        let coordinator = Arc::clone(&composition.coordinator);
        Self {
            ingress,
            workflow,
            external_conversation_id,
            binding,
            thread_scope,
            turn_scope,
            turn_store,
            coordinator,
            _product_harness: product_harness,
            thread_harness,
            model_gateway,
            capability_recorder,
            milestone_sink,
            worker: composition.worker,
            cancel: CancellationToken::new(),
            worker_tasks: Vec::new(),
            _turn_root: turn_root,
            _wake_sender: composition.wake_sender,
        }
    }

    pub fn start(&mut self) {
        self.start_workers(1);
    }

    pub fn start_workers(&mut self, count: usize) {
        if !self.worker_tasks.is_empty() {
            return;
        }
        for _ in 0..count.max(1) {
            let worker = Arc::clone(&self.worker);
            let cancel = self.cancel.clone();
            self.worker_tasks.push(tokio::spawn(async move {
                worker.run(cancel).await;
            }));
        }
    }

    pub async fn shutdown(&mut self) {
        self.cancel.cancel();
        for task in self.worker_tasks.drain(..) {
            let _ = task.await;
        }
    }

    pub async fn submit_text(&self, event_id: &str, text: &str) -> HarnessResult<SubmittedTurn> {
        self.submit_text_for(&self.external_conversation_id, "alice", event_id, text)
            .await
    }

    pub async fn submit_text_for(
        &self,
        conversation_id: &str,
        actor_id: &str,
        event_id: &str,
        text: &str,
    ) -> HarnessResult<SubmittedTurn> {
        self.submit_text_for_with_trigger(
            conversation_id,
            actor_id,
            event_id,
            text,
            ProductTriggerReason::DirectChat,
        )
        .await
    }

    pub async fn submit_text_for_with_trigger(
        &self,
        conversation_id: &str,
        actor_id: &str,
        event_id: &str,
        text: &str,
        trigger: ProductTriggerReason,
    ) -> HarnessResult<SubmittedTurn> {
        let envelope = self.ingress.verified_text_envelope_with_trigger(
            event_id,
            actor_id,
            conversation_id,
            text,
            trigger,
        )?;
        let binding_request = binding_request_from_envelope(&envelope);
        let route_kind = binding_request.route_kind;
        let binding = self
            ._product_harness
            .binding_service()?
            .resolve_binding(binding_request)
            .await?;
        let thread_scope = thread_scope_from_binding_with_route_kind(&binding, route_kind)?;
        let turn_scope = TurnScope::new_with_owner(
            binding.tenant_id.clone(),
            binding.agent_id.clone(),
            binding.project_id.clone(),
            binding.thread_id.clone(),
            binding.subject_user_id.clone(),
        );
        let actor = TurnActor::new(binding.actor_user_id.clone());
        let ack = self.workflow.accept_inbound(envelope).await?;
        let run_id = match &ack {
            ProductInboundAck::Accepted {
                submitted_run_id, ..
            } => *submitted_run_id,
            other => {
                return Err(format!("expected accepted inbound ack, got {other:?}").into());
            }
        };
        Ok(SubmittedTurn {
            ack,
            run_id,
            thread_id: binding.thread_id,
            thread_scope,
            scope: turn_scope,
            actor,
        })
    }

    pub async fn resume_blocked_turn(&self, run_id: TurnRunId) -> HarnessResult<()> {
        let blocked = self
            .run_state(run_id)
            .await?
            .gate_ref
            .ok_or("blocked run missing gate ref")?;
        self.resume_with_gate(run_id, blocked).await
    }

    pub async fn approve_and_resume_local_dev_gate(
        &self,
        run_id: TurnRunId,
    ) -> HarnessResult<GateRef> {
        let blocked = self
            .run_state(run_id)
            .await?
            .gate_ref
            .ok_or("blocked run missing gate ref")?;
        self.capability_recorder
            .approve_local_dev_gate(&blocked)
            .await?;
        self.resume_with_gate(run_id, blocked.clone()).await?;
        Ok(blocked)
    }

    pub async fn resume_blocked_turn_in_scope(
        &self,
        scope: TurnScope,
        actor: TurnActor,
        run_id: TurnRunId,
    ) -> HarnessResult<()> {
        let blocked = self
            .run_state_in_scope(scope.clone(), run_id)
            .await?
            .gate_ref
            .ok_or("blocked run missing gate ref")?;
        self.resume_with_gate_as(scope, actor, run_id, blocked, format!("resume-{run_id}"))
            .await
    }

    pub async fn resume_with_gate(
        &self,
        run_id: TurnRunId,
        gate_ref: GateRef,
    ) -> HarnessResult<()> {
        self.resume_with_gate_as(
            self.turn_scope.clone(),
            TurnActor::new(self.binding.actor_user_id.clone()),
            run_id,
            gate_ref,
            format!("resume-{run_id}"),
        )
        .await
    }

    pub async fn resume_with_gate_as(
        &self,
        scope: TurnScope,
        actor: TurnActor,
        run_id: TurnRunId,
        gate_ref: GateRef,
        idempotency_key: impl Into<String>,
    ) -> HarnessResult<()> {
        let response = self
            .coordinator
            .resume_turn(ResumeTurnRequest {
                scope,
                actor,
                run_id,
                gate_resolution_ref: gate_ref,
                precondition: ironclaw_turns::ResumeTurnPrecondition::AnyBlockedGate,
                source_binding_ref: SourceBindingRef::new("src:resume")?,
                reply_target_binding_ref: ReplyTargetBindingRef::new("reply:resume")?,
                idempotency_key: IdempotencyKey::new(idempotency_key.into())?,
            })
            .await?;
        if response.status != TurnStatus::Queued {
            return Err(format!("expected resumed run to queue, got {:?}", response.status).into());
        }
        Ok(())
    }

    pub async fn cancel_blocked_turn(&self, run_id: TurnRunId) -> HarnessResult<()> {
        self.cancel_run_as(
            self.turn_scope.clone(),
            TurnActor::new(self.binding.actor_user_id.clone()),
            run_id,
            format!("cancel-{run_id}"),
        )
        .await
    }

    pub async fn cancel_run_as(
        &self,
        scope: TurnScope,
        actor: TurnActor,
        run_id: TurnRunId,
        idempotency_key: impl Into<String>,
    ) -> HarnessResult<()> {
        let response = self
            .coordinator
            .cancel_run(CancelRunRequest {
                scope,
                actor,
                run_id,
                reason: SanitizedCancelReason::UserRequested,
                idempotency_key: IdempotencyKey::new(idempotency_key.into())?,
            })
            .await?;
        if !matches!(
            response.status,
            TurnStatus::Cancelled | TurnStatus::CancelRequested
        ) {
            return Err(format!(
                "expected run to be cancelled or cancel-requested, got {:?}",
                response.status
            )
            .into());
        }
        Ok(())
    }

    pub async fn wait_for_status(
        &self,
        run_id: TurnRunId,
        expected: TurnStatus,
    ) -> HarnessResult<TurnRunState> {
        self.wait_for_status_with_config(run_id, expected, WaitConfig::default())
            .await
    }

    pub async fn wait_for_status_with_config(
        &self,
        run_id: TurnRunId,
        expected: TurnStatus,
        wait: WaitConfig,
    ) -> HarnessResult<TurnRunState> {
        self.wait_for_status_in_scope_with_config(self.turn_scope.clone(), run_id, expected, wait)
            .await
    }

    pub async fn wait_for_submitted_status(
        &self,
        submitted: &SubmittedTurn,
        expected: TurnStatus,
    ) -> HarnessResult<TurnRunState> {
        self.wait_for_status_in_scope(submitted.scope.clone(), submitted.run_id, expected)
            .await
    }

    pub async fn wait_for_status_in_scope(
        &self,
        scope: TurnScope,
        run_id: TurnRunId,
        expected: TurnStatus,
    ) -> HarnessResult<TurnRunState> {
        self.wait_for_status_in_scope_with_config(scope, run_id, expected, WaitConfig::default())
            .await
    }

    pub async fn wait_for_status_in_scope_with_config(
        &self,
        scope: TurnScope,
        run_id: TurnRunId,
        expected: TurnStatus,
        wait: WaitConfig,
    ) -> HarnessResult<TurnRunState> {
        let deadline = tokio::time::Instant::now() + wait.timeout;
        loop {
            let state = self.run_state_in_scope(scope.clone(), run_id).await?;
            if state.status == expected {
                return Ok(state);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!(
                    "timed out waiting for {expected:?}; last status={:?} failure={:?}",
                    state.status, state.failure
                )
                .into());
            }
            tokio::time::sleep(wait.poll_interval).await;
        }
    }

    pub async fn run_state(&self, run_id: TurnRunId) -> HarnessResult<TurnRunState> {
        self.run_state_in_scope(self.turn_scope.clone(), run_id)
            .await
    }

    pub async fn run_state_in_scope(
        &self,
        scope: TurnScope,
        run_id: TurnRunId,
    ) -> HarnessResult<TurnRunState> {
        Ok(self
            .turn_store
            .get_run_state(GetRunStateRequest { scope, run_id })
            .await?)
    }

    pub async fn assert_final_reply(&self, text: &str) -> HarnessResult<()> {
        Ok(self
            .thread_harness
            .assert_final_reply(self.binding.thread_id.clone(), text)
            .await?)
    }

    pub async fn history(&self) -> HarnessResult<Vec<ThreadMessageRecord>> {
        self.history_for_thread(self.binding.thread_id.clone())
            .await
    }

    pub async fn history_for_submitted_thread(
        &self,
        submitted: &SubmittedTurn,
    ) -> HarnessResult<Vec<ThreadMessageRecord>> {
        self.history_for_thread_in_scope(
            submitted.thread_scope.clone(),
            submitted.thread_id.clone(),
        )
        .await
    }

    pub async fn history_for_thread(
        &self,
        thread_id: ThreadId,
    ) -> HarnessResult<Vec<ThreadMessageRecord>> {
        self.history_for_thread_in_scope(self.thread_scope.clone(), thread_id)
            .await
    }

    pub async fn history_for_thread_in_scope(
        &self,
        scope: ThreadScope,
        thread_id: ThreadId,
    ) -> HarnessResult<Vec<ThreadMessageRecord>> {
        Ok(self
            .thread_harness
            .service
            .list_thread_history(ThreadHistoryRequest { scope, thread_id })
            .await?
            .messages)
    }

    pub async fn children_of(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
    ) -> HarnessResult<Vec<TurnRunRecord>> {
        Ok(self.turn_store.children_of(scope, run_id).await?)
    }

    pub fn model_requests(&self) -> Vec<HostManagedModelRequest> {
        self.model_gateway.requests()
    }

    pub fn remaining_model_responses(&self) -> usize {
        self.model_gateway.remaining_responses()
    }

    pub fn assert_model_exhausted(&self) {
        self.model_gateway.assert_exhausted();
    }

    pub fn capability_invocations(&self) -> Vec<CapabilityInvocation> {
        self.capability_recorder.invocations()
    }

    pub fn capability_results(&self) -> Vec<RecordedCapabilityResult> {
        self.capability_recorder.capability_results()
    }

    pub fn runtime_http_requests(&self) -> Vec<RuntimeHttpEgressRequest> {
        self.capability_recorder.runtime_http_requests()
    }

    pub fn network_http_requests(&self) -> Vec<NetworkHttpRequest> {
        self.capability_recorder.network_http_requests()
    }

    pub fn host_workspace_file_path(&self, relative: &str) -> HarnessResult<PathBuf> {
        self.capability_recorder
            .workspace_file_path(relative)
            .ok_or_else(|| "harness is not using host-runtime capabilities".into())
    }

    pub fn milestones(&self) -> Vec<LoopHostMilestone> {
        self.milestone_sink.milestones()
    }
}

impl Drop for RebornBinaryE2EHarness {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

struct HarnessLoopExitEvidencePort {
    inner: ThreadCheckpointLoopExitEvidencePort<FilesystemSessionThreadService<LocalFilesystem>>,
    loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
    accept_harness_blocked_evidence: bool,
}

#[async_trait]
impl LoopExitEvidencePort for HarnessLoopExitEvidencePort {
    async fn verify_completion_refs(
        &self,
        request: CompletionEvidenceRequest<'_>,
    ) -> Result<bool, TurnError> {
        self.inner.verify_completion_refs(request).await
    }

    async fn verify_final_checkpoint(
        &self,
        request: FinalCheckpointEvidenceRequest<'_>,
    ) -> Result<bool, TurnError> {
        self.inner.verify_final_checkpoint(request).await
    }

    async fn verify_blocked_evidence(
        &self,
        request: BlockedEvidenceRequest<'_>,
    ) -> Result<bool, TurnError> {
        if self.inner.verify_blocked_evidence(request.clone()).await? {
            return Ok(true);
        }
        if !self.accept_harness_blocked_evidence {
            return Ok(false);
        }
        if !matches!(
            request.blocked.kind,
            LoopBlockedKind::Approval | LoopBlockedKind::AwaitDependentRun
        ) || GateRef::new(request.blocked.gate_ref.as_str()).is_err()
        {
            return Ok(false);
        }
        let checkpoint = self
            .loop_checkpoint_store
            .get_loop_checkpoint(GetLoopCheckpointRequest {
                scope: request.scope.clone(),
                turn_id: request.turn_id,
                run_id: request.run_id,
                checkpoint_id: request.blocked.checkpoint_id,
            })
            .await?;
        Ok(checkpoint
            .map(|record| {
                record.kind == LoopCheckpointKind::BeforeBlock
                    && record.state_ref == request.blocked.state_ref
            })
            .unwrap_or(false))
    }

    async fn verify_failure_evidence(
        &self,
        request: FailureEvidenceRequest<'_>,
    ) -> Result<bool, TurnError> {
        self.inner.verify_failure_evidence(request).await
    }

    async fn is_cancellation_observed(
        &self,
        scope: &TurnScope,
        turn_id: ironclaw_turns::TurnId,
        run_id: TurnRunId,
    ) -> Result<bool, TurnError> {
        self.inner
            .is_cancellation_observed(scope, turn_id, run_id)
            .await
    }

    async fn latest_checkpoint_kind(
        &self,
        scope: &TurnScope,
        turn_id: ironclaw_turns::TurnId,
        run_id: TurnRunId,
    ) -> Result<Option<ironclaw_turns::LoopCheckpointKind>, TurnError> {
        self.inner
            .latest_checkpoint_kind(scope, turn_id, run_id)
            .await
    }
}

impl HarnessCapabilityMode {
    fn into_parts(
        self,
        milestone_sink: Arc<ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink>,
    ) -> HarnessResult<HarnessCapabilityParts> {
        match self {
            Self::Recording(port) => {
                let port = Arc::new(port);
                let capability_io = Arc::new(ProductLiveCapabilityIo::default());
                Ok((
                    Arc::new(HarnessCapabilityPortFactory {
                        port: Arc::clone(&port),
                    }),
                    Arc::new(StaticCapabilitySurfaceProfileResolver {
                        allow_set: CapabilityAllowSet::allowlist(port.capability_allowlist()),
                    }),
                    capability_io.clone(),
                    capability_io,
                    HarnessCapabilityRecorder::Recording(port),
                ))
            }
            Self::HostRuntime(harness) => Ok((
                harness.capability_factory(milestone_sink),
                Arc::new(StaticCapabilitySurfaceProfileResolver {
                    allow_set: CapabilityAllowSet::allowlist(harness.capability_ids.clone()),
                }),
                harness.io.clone(),
                harness.capability_result_writer(),
                HarnessCapabilityRecorder::HostRuntime(harness),
            )),
        }
    }
}

struct HostRuntimeCapabilityHarness {
    runtime: Arc<dyn HostRuntime>,
    approval_parts: Option<RebornLocalDevApprovalTestParts>,
    pending_approval_scopes: Arc<Mutex<HashMap<ApprovalRequestId, ResourceScope>>>,
    io: Arc<ProductLiveCapabilityIo>,
    root: Arc<tempfile::TempDir>,
    workspace_root: PathBuf,
    mounts: MountView,
    capability_mount_overrides: Vec<(CapabilityId, MountView)>,
    capability_ids: Vec<CapabilityId>,
    runtime_kind: RuntimeKind,
    effect_kinds: Vec<EffectKind>,
    network_policy: NetworkPolicy,
    secrets: Vec<SecretHandle>,
    provider_id: ExtensionId,
    user_id: UserId,
    invocations: Arc<Mutex<Vec<CapabilityInvocation>>>,
    results: Arc<Mutex<Vec<RecordedCapabilityResult>>>,
    http_egress: Option<Arc<RecordingRuntimeHttpEgress>>,
    network_egress: Option<Arc<RecordingNetworkHttpEgress>>,
}

struct HostRuntimeHarnessOptions {
    mounts: MountView,
    runtime_policy: Option<ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy>,
}

impl HostRuntimeHarnessOptions {
    fn new(
        mounts: MountView,
        runtime_policy: Option<ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy>,
    ) -> Self {
        Self {
            mounts,
            runtime_policy,
        }
    }
}

impl HostRuntimeCapabilityHarness {
    async fn file_tools() -> HarnessResult<Self> {
        Self::file_tools_with_runtime_policy(Some(
            ironclaw_reborn_composition::local_dev_yolo_runtime_policy(true)?,
        ))
        .await
    }

    async fn file_tools_requiring_approval() -> HarnessResult<Self> {
        Self::file_tools_with_runtime_policy(None).await
    }

    async fn file_tools_with_runtime_policy(
        runtime_policy: Option<ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy>,
    ) -> HarnessResult<Self> {
        Self::new(
            "reborn-e2e-builtin-tools",
            vec![
                CapabilityId::new(WRITE_FILE_CAPABILITY_ID)?,
                CapabilityId::new(READ_FILE_CAPABILITY_ID)?,
            ],
            vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-builtin-user")?,
            runtime_policy,
        )
        .await
    }

    async fn write_only() -> HarnessResult<Self> {
        Self::new(
            "reborn-e2e-write-only",
            vec![CapabilityId::new(WRITE_FILE_CAPABILITY_ID)?],
            vec![EffectKind::WriteFilesystem],
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-write-only-user")?,
            None,
        )
        .await
    }

    async fn coding_read_tools() -> HarnessResult<Self> {
        Self::new(
            "reborn-e2e-coding-read-tools",
            vec![
                CapabilityId::new(LIST_DIR_CAPABILITY_ID)?,
                CapabilityId::new(GLOB_CAPABILITY_ID)?,
                CapabilityId::new(GREP_CAPABILITY_ID)?,
            ],
            vec![EffectKind::ReadFilesystem],
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-coding-read-user")?,
            None,
        )
        .await
    }

    async fn process_tools() -> HarnessResult<Self> {
        Self::new_with_options(
            "reborn-e2e-process-tools",
            vec![
                CapabilityId::new(ECHO_CAPABILITY_ID)?,
                CapabilityId::new(SHELL_CAPABILITY_ID)?,
                CapabilityId::new(SPAWN_SUBAGENT_CAPABILITY_ID)?,
            ],
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::Network,
                EffectKind::SpawnProcess,
                EffectKind::ExecuteCode,
            ],
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-process-user")?,
            HostRuntimeHarnessOptions::new(MountView::default(), None),
        )
        .await
    }

    async fn skill_management_tools() -> HarnessResult<Self> {
        let mut harness = Self::new_with_options(
            "reborn-e2e-skill-management-tools",
            vec![
                CapabilityId::new(SKILL_LIST_CAPABILITY_ID)?,
                CapabilityId::new(SKILL_INSTALL_CAPABILITY_ID)?,
                CapabilityId::new(SKILL_REMOVE_CAPABILITY_ID)?,
            ],
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::DeleteFilesystem,
                EffectKind::Network,
            ],
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-skill-management-user")?,
            HostRuntimeHarnessOptions::new(
                skill_mounts()?,
                Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                    true,
                )?),
            ),
        )
        .await?;
        harness.network_policy = http_test_policy();
        Ok(harness)
    }

    async fn trigger_management_tools() -> HarnessResult<Self> {
        Self::new_with_options(
            "reborn-e2e-trigger-management-tools",
            vec![
                CapabilityId::new(TRIGGER_CREATE_CAPABILITY_ID)?,
                CapabilityId::new(TRIGGER_LIST_CAPABILITY_ID)?,
                CapabilityId::new(TRIGGER_REMOVE_CAPABILITY_ID)?,
            ],
            vec![EffectKind::DispatchCapability, EffectKind::ExternalWrite],
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-trigger-management-user")?,
            HostRuntimeHarnessOptions::new(
                MountView::default(),
                Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                    true,
                )?),
            ),
        )
        .await
    }

    async fn new(
        service_label: &'static str,
        capability_ids: Vec<CapabilityId>,
        effect_kinds: Vec<EffectKind>,
        secrets: Vec<SecretHandle>,
        provider_id: ExtensionId,
        user_id: UserId,
        runtime_policy: Option<ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy>,
    ) -> HarnessResult<Self> {
        Self::new_with_options(
            service_label,
            capability_ids,
            effect_kinds,
            secrets,
            provider_id,
            user_id,
            HostRuntimeHarnessOptions::new(
                workspace_mounts(MountPermissions::read_write_list_delete())?,
                runtime_policy,
            ),
        )
        .await
    }

    async fn new_with_options(
        service_label: &'static str,
        capability_ids: Vec<CapabilityId>,
        effect_kinds: Vec<EffectKind>,
        secrets: Vec<SecretHandle>,
        provider_id: ExtensionId,
        user_id: UserId,
        options: HostRuntimeHarnessOptions,
    ) -> HarnessResult<Self> {
        let HostRuntimeHarnessOptions {
            mounts,
            runtime_policy,
        } = options;
        let root = Arc::new(tempfile::tempdir()?);
        let storage_root = root.path().join("local-dev");
        let workspace_root = storage_root.join("workspace");
        std::fs::create_dir_all(&workspace_root)?;
        let mut input = if runtime_policy.as_ref().is_some_and(|policy| {
            policy.resolved_profile == ironclaw_host_api::runtime_policy::RuntimeProfile::LocalYolo
        }) {
            let host_home_root = root.path().join("host-home");
            std::fs::create_dir_all(&host_home_root)?;
            ironclaw_reborn_composition::local_runtime_build_input_with_options(
                ironclaw_reborn_composition::RebornCompositionProfile::LocalDevYolo,
                service_label,
                storage_root,
                ironclaw_reborn_composition::RebornLocalRuntimeProfileOptions {
                    confirm_host_access: true,
                },
            )?
            .with_local_dev_confirmed_host_home_root(host_home_root)
        } else {
            RebornBuildInput::local_dev(service_label, storage_root)
        };
        if let Some(runtime_policy) = runtime_policy {
            input = input.with_runtime_policy(runtime_policy);
        }
        let services = build_reborn_services(input).await?;
        let approval_parts = services.local_dev_approval_test_parts();
        let pending_approval_scopes = Arc::new(Mutex::new(HashMap::new()));
        let runtime = services
            .host_runtime
            .ok_or("local-dev Reborn services missing host runtime")?;
        let runtime = Arc::new(RecordingHostRuntime::new(
            runtime,
            Arc::clone(&pending_approval_scopes),
        ));
        Ok(Self {
            runtime,
            approval_parts,
            pending_approval_scopes,
            io: Arc::new(ProductLiveCapabilityIo::default()),
            root,
            workspace_root,
            mounts,
            capability_mount_overrides: Vec::new(),
            capability_ids,
            runtime_kind: RuntimeKind::FirstParty,
            effect_kinds,
            network_policy: NetworkPolicy::default(),
            secrets,
            provider_id,
            user_id,
            invocations: Arc::new(Mutex::new(Vec::new())),
            results: Arc::new(Mutex::new(Vec::new())),
            http_egress: None,
            network_egress: None,
        })
    }

    async fn core_builtin_tools() -> HarnessResult<Self> {
        Self::core_builtin_tools_with_network_policy(http_test_policy()).await
    }

    async fn core_builtin_tools_with_network_policy(
        network_policy: NetworkPolicy,
    ) -> HarnessResult<Self> {
        let (root, storage_root, workspace_root) = host_runtime_storage_roots()?;
        let runtime_http_egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
            br#"{"accepted":true}"#.to_vec(),
        ));
        let runtime = local_dev_host_runtime_with_http_egress(
            storage_root.clone(),
            Arc::clone(&runtime_http_egress),
        )?;
        let mut harness = Self::core_builtin_tools_from_runtime(
            root,
            workspace_root,
            runtime,
            network_policy,
            UserId::new("reborn-e2e-core-builtins-user")?,
        )?;
        harness.http_egress = Some(runtime_http_egress);
        Ok(harness)
    }

    async fn core_builtin_tools_with_live_http_egress(
        network_policy: NetworkPolicy,
    ) -> HarnessResult<Self> {
        let (root, storage_root, workspace_root) = host_runtime_storage_roots()?;
        let runtime = local_dev_host_runtime_with_live_http_egress(storage_root.clone())?;
        Self::core_builtin_tools_from_runtime(
            root,
            workspace_root,
            runtime,
            network_policy,
            UserId::new("reborn-e2e-core-builtins-live-http-user")?,
        )
    }

    fn core_builtin_tools_from_runtime(
        root: Arc<tempfile::TempDir>,
        workspace_root: PathBuf,
        runtime: Arc<dyn HostRuntime>,
        network_policy: NetworkPolicy,
        user_id: UserId,
    ) -> HarnessResult<Self> {
        let mounts = workspace_mounts(MountPermissions::read_write_list_delete())?;
        let memory_mounts = memory_mounts(MountPermissions::read_write_list_delete())?;
        let memory_capability_ids = [
            CapabilityId::new(MEMORY_SEARCH_CAPABILITY_ID)?,
            CapabilityId::new(MEMORY_WRITE_CAPABILITY_ID)?,
            CapabilityId::new(MEMORY_READ_CAPABILITY_ID)?,
            CapabilityId::new(MEMORY_TREE_CAPABILITY_ID)?,
        ];
        Ok(Self {
            runtime,
            approval_parts: None,
            pending_approval_scopes: Arc::new(Mutex::new(HashMap::new())),
            io: Arc::new(ProductLiveCapabilityIo::default()),
            root,
            workspace_root,
            mounts,
            capability_mount_overrides: memory_capability_ids
                .iter()
                .cloned()
                .map(|capability_id| (capability_id, memory_mounts.clone()))
                .collect(),
            capability_ids: vec![
                CapabilityId::new(TIME_CAPABILITY_ID)?,
                CapabilityId::new(JSON_CAPABILITY_ID)?,
                CapabilityId::new(HTTP_CAPABILITY_ID)?,
                CapabilityId::new(HTTP_SAVE_CAPABILITY_ID)?,
                CapabilityId::new(MEMORY_SEARCH_CAPABILITY_ID)?,
                CapabilityId::new(MEMORY_WRITE_CAPABILITY_ID)?,
                CapabilityId::new(MEMORY_READ_CAPABILITY_ID)?,
                CapabilityId::new(MEMORY_TREE_CAPABILITY_ID)?,
                CapabilityId::new(READ_FILE_CAPABILITY_ID)?,
                CapabilityId::new(APPLY_PATCH_CAPABILITY_ID)?,
            ],
            runtime_kind: RuntimeKind::FirstParty,
            effect_kinds: vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::Network,
            ],
            network_policy,
            secrets: Vec::new(),
            provider_id: ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            user_id,
            invocations: Arc::new(Mutex::new(Vec::new())),
            results: Arc::new(Mutex::new(Vec::new())),
            http_egress: None,
            network_egress: None,
        })
    }

    async fn github_issue_tools() -> HarnessResult<Self> {
        let root = Arc::new(tempfile::tempdir()?);
        let storage_root = root.path().join("local-dev");
        let workspace_root = storage_root.join("workspace");
        std::fs::create_dir_all(&workspace_root)?;
        let github_fixture_response =
            br#"{"object":{"sha":"abc123def4567890abc123def4567890abc123de"},"ok":true}"#.to_vec();
        let runtime_http_egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
            github_fixture_response.clone(),
        ));
        let network_egress = Arc::new(RecordingNetworkHttpEgress::with_body(
            github_fixture_response,
        ));
        let runtime = local_dev_host_runtime_with_registry_and_egress(
            storage_root.clone(),
            github_support::extension_registry()?,
            runtime_http_egress.clone(),
            network_egress.clone(),
        )?;
        let mounts = workspace_mounts(MountPermissions::read_write_list_delete())?;
        Ok(Self {
            runtime,
            approval_parts: None,
            pending_approval_scopes: Arc::new(Mutex::new(HashMap::new())),
            io: Arc::new(ProductLiveCapabilityIo::default()),
            root,
            workspace_root,
            mounts,
            capability_mount_overrides: Vec::new(),
            capability_ids: github_support::capability_ids()?,
            runtime_kind: RuntimeKind::Wasm,
            effect_kinds: github_support::effect_kinds(),
            network_policy: github_support::api_policy(),
            secrets: github_support::secret_handles()?,
            provider_id: github_support::provider_id()?,
            user_id: UserId::new("reborn-e2e-github-user")?,
            invocations: Arc::new(Mutex::new(Vec::new())),
            results: Arc::new(Mutex::new(Vec::new())),
            http_egress: Some(runtime_http_egress),
            network_egress: Some(network_egress),
        })
    }

    fn capability_factory(
        self: &Arc<Self>,
        milestone_sink: Arc<ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink>,
    ) -> Arc<dyn LoopCapabilityPortFactory> {
        Arc::new(HostRuntimeHarnessCapabilityPortFactory {
            harness: Arc::clone(self),
            milestone_sink,
        })
    }

    fn capability_result_writer(self: &Arc<Self>) -> Arc<dyn LoopCapabilityResultWriter> {
        Arc::new(RecordingCapabilityResultWriter {
            inner: self.io.clone(),
            results: Arc::clone(&self.results),
        })
    }

    fn invocations(&self) -> Vec<CapabilityInvocation> {
        self.invocations.lock().unwrap().clone()
    }

    fn capability_results(&self) -> Vec<RecordedCapabilityResult> {
        self.results.lock().unwrap().clone()
    }

    fn runtime_http_requests(&self) -> Vec<RuntimeHttpEgressRequest> {
        self.http_egress
            .as_ref()
            .map(|egress| egress.requests())
            .unwrap_or_default()
    }

    fn network_http_requests(&self) -> Vec<NetworkHttpRequest> {
        self.network_egress
            .as_ref()
            .map(|egress| egress.requests())
            .unwrap_or_default()
    }

    fn workspace_file_path(&self, relative: &str) -> PathBuf {
        self.workspace_root.join(relative.trim_start_matches('/'))
    }

    async fn approve_local_dev_gate(&self, gate_ref: &GateRef) -> HarnessResult<()> {
        let approval_parts = self
            .approval_parts
            .as_ref()
            .ok_or("host runtime harness has no local-dev approval stores")?;
        let request_id = approval_request_id_from_gate_ref(gate_ref)?;
        let scope = self
            .pending_approval_scopes
            .lock()
            .unwrap()
            .get(&request_id)
            .cloned()
            .ok_or("approval gate was not recorded by the host runtime harness")?;
        let record = approval_parts
            .approval_requests
            .get(&scope, request_id)
            .await?
            .ok_or("approval request was not persisted")?;
        let capability = match record.request.action.as_ref() {
            Action::Dispatch { capability, .. } | Action::SpawnCapability { capability, .. } => {
                capability.clone()
            }
            other => return Err(format!("unsupported approval action: {other:?}").into()),
        };
        let approval = self.lease_approval_for(&capability);
        let resolver = ApprovalResolver::new(
            approval_parts.approval_requests.as_ref(),
            approval_parts.capability_leases.as_ref(),
        );
        match record.request.action.as_ref() {
            Action::Dispatch { .. } => {
                resolver
                    .approve_dispatch(&scope, request_id, approval)
                    .await?;
            }
            Action::SpawnCapability { .. } => {
                resolver.approve_spawn(&scope, request_id, approval).await?;
            }
            other => return Err(format!("unsupported approval action: {other:?}").into()),
        }
        Ok(())
    }

    fn lease_approval_for(&self, capability_id: &CapabilityId) -> LeaseApproval {
        let mounts = self
            .capability_mount_overrides
            .iter()
            .find(|(override_capability, _)| override_capability == capability_id)
            .map(|(_, mounts)| mounts.clone())
            .unwrap_or_else(|| self.mounts.clone());
        LeaseApproval {
            issued_by: Principal::HostRuntime,
            allowed_effects: self.effect_kinds.clone(),
            mounts,
            network: self.network_policy.clone(),
            secrets: self.secrets.clone(),
            resource_ceiling: None,
            expires_at: None,
            max_invocations: Some(1),
        }
    }
}

fn approval_request_id_from_gate_ref(gate_ref: &GateRef) -> HarnessResult<ApprovalRequestId> {
    const APPROVAL_GATE_PREFIX: &str = "gate:approval-";
    let value = gate_ref
        .as_str()
        .strip_prefix(APPROVAL_GATE_PREFIX)
        .ok_or("gate ref is not a local-dev approval gate")?;
    Ok(ApprovalRequestId::parse(value)?)
}

struct RecordingHostRuntime {
    inner: Arc<dyn HostRuntime>,
    pending_approval_scopes: Arc<Mutex<HashMap<ApprovalRequestId, ResourceScope>>>,
}

impl RecordingHostRuntime {
    fn new(
        inner: Arc<dyn HostRuntime>,
        pending_approval_scopes: Arc<Mutex<HashMap<ApprovalRequestId, ResourceScope>>>,
    ) -> Self {
        Self {
            inner,
            pending_approval_scopes,
        }
    }
}

#[async_trait]
impl HostRuntime for RecordingHostRuntime {
    async fn invoke_capability(
        &self,
        request: RuntimeCapabilityRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        let scope = request.context.resource_scope.clone();
        let outcome = self.inner.invoke_capability(request).await?;
        if let RuntimeCapabilityOutcome::ApprovalRequired(gate) = &outcome {
            self.pending_approval_scopes
                .lock()
                .unwrap()
                .insert(gate.approval_request_id, scope);
        }
        Ok(outcome)
    }

    async fn spawn_capability(
        &self,
        request: RuntimeCapabilityRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        let scope = request.context.resource_scope.clone();
        let outcome = self.inner.spawn_capability(request).await?;
        if let RuntimeCapabilityOutcome::ApprovalRequired(gate) = &outcome {
            self.pending_approval_scopes
                .lock()
                .unwrap()
                .insert(gate.approval_request_id, scope);
        }
        Ok(outcome)
    }

    async fn resume_capability(
        &self,
        request: RuntimeCapabilityResumeRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        self.inner.resume_capability(request).await
    }

    async fn resume_spawn_capability(
        &self,
        request: RuntimeCapabilityResumeRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        self.inner.resume_spawn_capability(request).await
    }

    async fn visible_capabilities(
        &self,
        request: RuntimeVisibleCapabilityRequest,
    ) -> Result<RuntimeVisibleCapabilitySurface, HostRuntimeError> {
        self.inner.visible_capabilities(request).await
    }

    async fn cancel_work(
        &self,
        request: CancelRuntimeWorkRequest,
    ) -> Result<CancelRuntimeWorkOutcome, HostRuntimeError> {
        self.inner.cancel_work(request).await
    }

    async fn runtime_status(
        &self,
        request: RuntimeStatusRequest,
    ) -> Result<HostRuntimeStatus, HostRuntimeError> {
        self.inner.runtime_status(request).await
    }

    async fn health(&self) -> Result<HostRuntimeHealth, HostRuntimeError> {
        self.inner.health().await
    }
}

struct HostRuntimeHarnessCapabilityPortFactory {
    harness: Arc<HostRuntimeCapabilityHarness>,
    milestone_sink: Arc<ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink>,
}

#[async_trait]
impl LoopCapabilityPortFactory for HostRuntimeHarnessCapabilityPortFactory {
    async fn create_capability_port(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        let authority = ProductLiveVisibleCapabilityRequestConfig::new(
            self.harness.user_id.clone(),
            self.harness.runtime_kind,
            TrustClass::FirstParty,
            SurfaceKind::new("agent_loop").map_err(host_runtime_harness_error)?,
            CapabilitySurfacePolicy::allow_all(),
        )
        .with_mounts(self.harness.mounts.clone())
        .with_grants(capability_grants(
            Principal::User(self.harness.user_id.clone()),
            &self.harness.capability_ids,
            self.harness.effect_kinds.clone(),
            self.harness.mounts.clone(),
            &self.harness.capability_mount_overrides,
            self.harness.network_policy.clone(),
            self.harness.secrets.clone(),
        ))
        .with_provider_trust_for_effects(
            self.harness.provider_id.clone(),
            EffectiveTrustClass::user_trusted(),
            self.harness.effect_kinds.clone(),
        );
        let execution_mounts = self.harness.mounts.clone();
        let visible_request = visible_capability_request_for_run(run_context, authority)
            .map_err(host_runtime_harness_error)?;
        let milestone_sink: Arc<dyn LoopHostMilestoneSink> = self.milestone_sink.clone();
        let result_writer = Arc::new(RecordingCapabilityResultWriter {
            inner: self.harness.io.clone(),
            results: Arc::clone(&self.harness.results),
        });
        let mut factory = HostRuntimeLoopCapabilityPortFactory::new(
            Arc::clone(&self.harness.runtime),
            visible_request,
            self.harness.io.clone(),
            result_writer,
            milestone_sink,
        )
        .with_execution_mounts(execution_mounts);
        for (capability_id, mounts) in &self.harness.capability_mount_overrides {
            factory =
                factory.with_capability_execution_mount(capability_id.clone(), mounts.clone());
        }
        let port = factory.for_run_context(run_context.clone());
        Ok(Arc::new(RecordingDelegatingCapabilityPort {
            inner: port,
            invocations: Arc::clone(&self.harness.invocations),
        }))
    }
}

struct RecordingDelegatingCapabilityPort {
    inner: Arc<dyn LoopCapabilityPort>,
    invocations: Arc<Mutex<Vec<CapabilityInvocation>>>,
}

#[async_trait]
impl LoopCapabilityPort for RecordingDelegatingCapabilityPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        self.inner.tool_definitions()
    }

    fn validate_provider_tool_call(
        &self,
        tool_call: &ProviderToolCall,
    ) -> Result<(), AgentLoopHostError> {
        self.inner.validate_provider_tool_call(tool_call)
    }

    async fn register_provider_tool_call(
        &self,
        tool_call: ProviderToolCall,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        self.inner.register_provider_tool_call(tool_call).await
    }

    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        self.inner.visible_capabilities(request).await
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        self.invocations.lock().unwrap().push(request.clone());
        self.inner.invoke_capability(request).await
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        self.invocations
            .lock()
            .unwrap()
            .extend(request.invocations.iter().cloned());
        self.inner.invoke_capability_batch(request).await
    }
}

fn local_dev_host_runtime_with_http_egress(
    storage_root: PathBuf,
    egress: Arc<RecordingRuntimeHttpEgress>,
) -> HarnessResult<Arc<dyn HostRuntime>> {
    let mut registry = ExtensionRegistry::new();
    registry.insert(builtin_first_party_package()?)?;
    local_dev_host_runtime_with_registry_and_runtime_http_egress(storage_root, registry, egress)
}

fn host_runtime_storage_roots() -> HarnessResult<(Arc<tempfile::TempDir>, PathBuf, PathBuf)> {
    let root = Arc::new(tempfile::tempdir()?);
    let storage_root = root.path().join("local-dev");
    let workspace_root = storage_root.join("workspace");
    std::fs::create_dir_all(&workspace_root)?;
    Ok((root, storage_root, workspace_root))
}

fn local_dev_host_runtime_with_registry_and_runtime_http_egress(
    storage_root: PathBuf,
    registry: ExtensionRegistry,
    egress: Arc<RecordingRuntimeHttpEgress>,
) -> HarnessResult<Arc<dyn HostRuntime>> {
    let services = HostRuntimeServices::new(
        Arc::new(registry),
        local_dev_root_filesystem(storage_root, LocalDevRootMounts::core_builtins())?,
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        HostRuntimeCapabilitySurfaceVersion::new("reborn-app-v1")?,
    )
    .with_secret_store(Arc::new(StaticSecretStore::new(
        SecretHandle::new("github_manual_access")?,
        SecretMaterial::from("ghp_fake_fixture_token"),
    )))
    .with_runtime_credential_account_resolver(Arc::new(FixedRuntimeCredentialAccountResolver {
        result: Ok(SecretHandle::new("github_manual_access")?),
    }))
    .with_first_party_capabilities(Arc::new(builtin_first_party_handlers(Arc::new(
        ironclaw_triggers::InMemoryTriggerRepository::default(),
    ))?))
    .with_first_party_http_egress(egress)
    .with_trust_policy(Arc::new(first_party_trust_policy()?));

    Ok(Arc::new(services.host_runtime_for_local_testing()))
}

fn local_dev_host_runtime_with_registry_and_egress(
    storage_root: PathBuf,
    registry: ExtensionRegistry,
    runtime_http_egress: Arc<RecordingRuntimeHttpEgress>,
    network_egress: Arc<RecordingNetworkHttpEgress>,
) -> HarnessResult<Arc<dyn HostRuntime>> {
    let services = HostRuntimeServices::new(
        Arc::new(registry),
        local_dev_root_filesystem(storage_root, LocalDevRootMounts::github_assets())?,
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GithubHarnessAuthorizer::new()?),
        ironclaw_processes::ProcessServices::in_memory(),
        HostRuntimeCapabilitySurfaceVersion::new("reborn-app-v1")?,
    )
    .with_secret_store(Arc::new(StaticSecretStore::new(
        SecretHandle::new("github_manual_access")?,
        SecretMaterial::from("ghp_fake_fixture_token"),
    )))
    .with_runtime_credential_account_resolver(Arc::new(FixedRuntimeCredentialAccountResolver {
        result: Ok(SecretHandle::new("github_manual_access")?),
    }))
    .with_first_party_capabilities(Arc::new(builtin_first_party_handlers(Arc::new(
        ironclaw_triggers::InMemoryTriggerRepository::default(),
    ))?))
    .with_runtime_http_egress(runtime_http_egress)
    .with_trust_policy(Arc::new(github_first_party_trust_policy()?))
    .try_with_host_http_egress((*network_egress).clone())
    .map_err(|report| std::io::Error::other(format!("host HTTP egress failed: {report:?}")))?
    .try_with_wasm_runtime(WitToolRuntimeConfig::default(), WitToolHost::deny_all())
    .map_err(|report| std::io::Error::other(format!("WASM runtime failed: {report:?}")))?;

    Ok(Arc::new(services.host_runtime_for_local_testing()))
}

fn local_dev_host_runtime_with_live_http_egress(
    storage_root: PathBuf,
) -> HarnessResult<Arc<dyn HostRuntime>> {
    let mut registry = ExtensionRegistry::new();
    registry.insert(builtin_first_party_package()?)?;

    let services = HostRuntimeServices::new(
        Arc::new(registry),
        local_dev_root_filesystem(storage_root, LocalDevRootMounts::core_builtins())?,
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ironclaw_processes::ProcessServices::in_memory(),
        HostRuntimeCapabilitySurfaceVersion::new("reborn-app-v1")?,
    )
    .with_secret_store(Arc::new(InMemorySecretStore::new()))
    .with_first_party_capabilities(Arc::new(builtin_first_party_handlers(Arc::new(
        ironclaw_triggers::InMemoryTriggerRepository::default(),
    ))?))
    .try_with_host_http_egress(PolicyNetworkHttpEgress::new(ReqwestNetworkTransport::new(
        Duration::from_secs(2),
    )))
    .map_err(|report| {
        std::io::Error::other(format!(
            "live HTTP egress production wiring failed: {report:?}"
        ))
    })?
    .with_trust_policy(Arc::new(first_party_trust_policy()?));

    Ok(Arc::new(services.host_runtime_for_local_testing()))
}

fn local_dev_root_filesystem(
    storage_root: PathBuf,
    mounts: LocalDevRootMounts,
) -> HarnessResult<Arc<CompositeRootFilesystem>> {
    let mut local = LocalFilesystem::new();
    local.mount_local(
        VirtualPath::new("/projects")?,
        HostPath::from_path_buf(storage_root),
    )?;
    if mounts.github_assets {
        local.mount_local(
            VirtualPath::new("/system/extensions/github")?,
            HostPath::from_path_buf(github_support::asset_root()),
        )?;
    }

    let local = Arc::new(local);
    let mut root = CompositeRootFilesystem::new();
    root.mount(
        local_dev_mount_descriptor(
            "/projects",
            "local-dev-projects",
            BackendKind::LocalFilesystem,
            StorageClass::FileContent,
            ContentKind::ProjectFile,
            IndexPolicy::NotIndexed,
            BackendCapabilities::bytes_only(),
        )?,
        Arc::clone(&local),
    )?;
    if mounts.github_assets {
        root.mount(
            local_dev_mount_descriptor(
                "/system/extensions/github",
                "local-dev-github-assets",
                BackendKind::LocalFilesystem,
                StorageClass::FileContent,
                ContentKind::ExtensionPackage,
                IndexPolicy::NotIndexed,
                BackendCapabilities::bytes_only(),
            )?,
            Arc::clone(&local),
        )?;
    }
    if mounts.memory {
        let memory = Arc::new(InMemoryBackend::new());
        root.mount(
            local_dev_mount_descriptor(
                "/memory",
                "local-dev-memory",
                BackendKind::MemoryDocuments,
                StorageClass::StructuredRecords,
                ContentKind::MemoryDocument,
                IndexPolicy::FullTextAndVector,
                memory.capabilities(),
            )?,
            memory,
        )?;
    }
    Ok(Arc::new(root))
}

#[derive(Clone, Copy)]
struct LocalDevRootMounts {
    github_assets: bool,
    memory: bool,
}

impl LocalDevRootMounts {
    fn core_builtins() -> Self {
        Self {
            github_assets: false,
            memory: true,
        }
    }

    fn github_assets() -> Self {
        Self {
            github_assets: true,
            memory: false,
        }
    }
}

fn local_dev_mount_descriptor(
    virtual_root: &str,
    backend_id: &str,
    backend_kind: BackendKind,
    storage_class: StorageClass,
    content_kind: ContentKind,
    index_policy: IndexPolicy,
    capabilities: BackendCapabilities,
) -> HarnessResult<MountDescriptor> {
    Ok(MountDescriptor {
        virtual_root: VirtualPath::new(virtual_root)?,
        backend_id: BackendId::new(backend_id)?,
        backend_kind,
        storage_class,
        content_kind,
        index_policy,
        capabilities,
    })
}

fn first_party_trust_policy() -> HarnessResult<HostTrustPolicy> {
    Ok(HostTrustPolicy::new(vec![Box::new(
        AdminConfig::with_entries(vec![AdminEntry::for_local_manifest(
            PackageId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            "/system/extensions/builtin/manifest.toml".to_string(),
            None,
            HostTrustAssignment::first_party(),
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::Network,
            ],
            None,
        )]),
    )])?)
}

fn github_first_party_trust_policy() -> HarnessResult<HostTrustPolicy> {
    Ok(HostTrustPolicy::new(vec![Box::new(
        AdminConfig::with_entries(vec![AdminEntry::for_local_manifest(
            PackageId::new("github")?,
            "/system/extensions/github/manifest.toml".to_string(),
            None,
            HostTrustAssignment::first_party(),
            vec![
                EffectKind::DispatchCapability,
                EffectKind::Network,
                EffectKind::UseSecret,
                EffectKind::ExternalWrite,
            ],
            None,
        )]),
    )])?)
}

fn http_test_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: Some(NetworkScheme::Https),
            host_pattern: "api.example.test".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(10_000),
    }
}

#[derive(Debug)]
struct FixedRuntimeCredentialAccountResolver {
    result: Result<SecretHandle, CredentialStageError>,
}

#[async_trait]
impl RuntimeCredentialAccountResolver for FixedRuntimeCredentialAccountResolver {
    async fn resolve_access_secret(
        &self,
        request: RuntimeCredentialAccountRequest<'_>,
    ) -> Result<RuntimeCredentialAccessSecret, CredentialStageError> {
        assert_eq!(request.provider.as_str(), "github");
        assert_eq!(request.requester_extension.as_str(), "github");
        self.result
            .clone()
            .map(|handle| RuntimeCredentialAccessSecret {
                scope: request.scope.clone(),
                handle,
            })
    }
}

struct StaticSecretStore {
    handle: SecretHandle,
    material: SecretMaterial,
}

impl StaticSecretStore {
    fn new(handle: SecretHandle, material: SecretMaterial) -> Self {
        Self { handle, material }
    }
}

#[async_trait]
impl SecretStore for StaticSecretStore {
    async fn put(
        &self,
        scope: ResourceScope,
        handle: SecretHandle,
        _material: SecretMaterial,
    ) -> Result<SecretMetadata, SecretStoreError> {
        Ok(SecretMetadata { scope, handle })
    }

    async fn metadata(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMetadata>, SecretStoreError> {
        Ok((handle == &self.handle).then(|| SecretMetadata {
            scope: scope.clone(),
            handle: handle.clone(),
        }))
    }

    async fn delete(
        &self,
        _scope: &ResourceScope,
        _handle: &SecretHandle,
    ) -> Result<bool, SecretStoreError> {
        Ok(false)
    }

    async fn lease_once(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretLease, SecretStoreError> {
        if handle != &self.handle {
            return Err(SecretStoreError::UnknownSecret {
                scope: Box::new(scope.clone()),
                handle: handle.clone(),
            });
        }
        Ok(SecretLease {
            id: SecretLeaseId::new(),
            scope: scope.clone(),
            handle: handle.clone(),
            status: SecretLeaseStatus::Active,
        })
    }

    async fn consume(
        &self,
        _scope: &ResourceScope,
        _lease_id: SecretLeaseId,
    ) -> Result<SecretMaterial, SecretStoreError> {
        Ok(self.material.clone())
    }

    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretLease, SecretStoreError> {
        Ok(SecretLease {
            id: lease_id,
            scope: scope.clone(),
            handle: self.handle.clone(),
            status: SecretLeaseStatus::Revoked,
        })
    }

    async fn leases_for_scope(
        &self,
        _scope: &ResourceScope,
    ) -> Result<Vec<SecretLease>, SecretStoreError> {
        Ok(Vec::new())
    }
}

struct GithubHarnessAuthorizer {
    obligations: Obligations,
}

impl GithubHarnessAuthorizer {
    fn new() -> HarnessResult<Self> {
        Ok(Self {
            obligations: Obligations::new(vec![
                Obligation::ApplyNetworkPolicy {
                    policy: github_support::api_policy(),
                },
                Obligation::InjectCredentialAccountOnce {
                    handle: SecretHandle::new("github_runtime_token")?,
                    provider: RuntimeCredentialAccountProviderId::new("github")?,
                    setup: ironclaw_host_api::RuntimeCredentialAccountSetup::ManualToken,
                    provider_scopes: Vec::new(),
                    requester_extension: ExtensionId::new("github")?,
                },
            ])?,
        })
    }
}

#[async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for GithubHarnessAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: self.obligations.clone(),
        }
    }

    async fn authorize_spawn_with_trust(
        &self,
        _context: &ExecutionContext,
        _descriptor: &CapabilityDescriptor,
        _estimate: &ResourceEstimate,
        _trust_decision: &TrustDecision,
    ) -> Decision {
        Decision::Allow {
            obligations: self.obligations.clone(),
        }
    }
}

#[derive(Debug, Clone)]
struct RecordingRuntimeHttpEgress {
    default_body: Vec<u8>,
    response_bodies: Arc<Mutex<VecDeque<Vec<u8>>>>,
    requests: Arc<Mutex<Vec<RuntimeHttpEgressRequest>>>,
}

impl RecordingRuntimeHttpEgress {
    fn with_body(body: Vec<u8>) -> Self {
        Self {
            default_body: body,
            response_bodies: Arc::new(Mutex::new(VecDeque::new())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<RuntimeHttpEgressRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl RuntimeHttpEgress for RecordingRuntimeHttpEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        let request_bytes = request.body.len() as u64;
        self.requests.lock().unwrap().push(request);
        let body = self
            .response_bodies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| self.default_body.clone());
        Ok(RuntimeHttpEgressResponse {
            status: 200,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: body.clone(),
            saved_body: None,
            request_bytes,
            response_bytes: body.len() as u64,
            redaction_applied: false,
        })
    }
}

#[async_trait]
impl ironclaw_host_runtime::ToolCallHttpEgress for RecordingRuntimeHttpEgress {
    async fn execute_for_model_visible_output(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        RuntimeHttpEgress::execute(self, request).await
    }
}

#[derive(Debug, Clone)]
struct RecordingNetworkHttpEgress {
    default_body: Vec<u8>,
    response_bodies: Arc<Mutex<VecDeque<Vec<u8>>>>,
    requests: Arc<Mutex<Vec<NetworkHttpRequest>>>,
}

impl RecordingNetworkHttpEgress {
    fn with_body(body: Vec<u8>) -> Self {
        Self {
            default_body: body,
            response_bodies: Arc::new(Mutex::new(VecDeque::new())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<NetworkHttpRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl NetworkHttpEgress for RecordingNetworkHttpEgress {
    async fn execute(
        &self,
        request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        let request_bytes = request.body.len() as u64;
        self.requests.lock().unwrap().push(request);
        let body = self
            .response_bodies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| self.default_body.clone());
        Ok(NetworkHttpResponse {
            status: 200,
            headers: vec![("content-type".to_string(), "application/json".to_string())],
            body: body.clone(),
            usage: NetworkUsage {
                request_bytes,
                response_bytes: body.len() as u64,
                resolved_ip: None,
            },
        })
    }
}

struct RecordingCapabilityResultWriter {
    inner: Arc<ProductLiveCapabilityIo>,
    results: Arc<Mutex<Vec<RecordedCapabilityResult>>>,
}

#[async_trait]
impl LoopCapabilityResultWriter for RecordingCapabilityResultWriter {
    async fn write_capability_result(
        &self,
        write: CapabilityResultWrite<'_>,
    ) -> Result<(LoopResultRef, u64), AgentLoopHostError> {
        let capability_id = write.capability_id.clone();
        let output = write.output.clone();
        let (result_ref, byte_len) = self.inner.write_capability_result(write).await?;
        self.results.lock().unwrap().push(RecordedCapabilityResult {
            capability_id,
            output,
        });
        Ok((result_ref, byte_len))
    }

    async fn update_capability_result(
        &self,
        run_context: &LoopRunContext,
        result_ref: &LoopResultRef,
        output: serde_json::Value,
    ) -> Result<u64, AgentLoopHostError> {
        let byte_len = self
            .inner
            .update_capability_result(run_context, result_ref, output.clone())
            .await?;
        self.results.lock().unwrap().push(RecordedCapabilityResult {
            capability_id: CapabilityId::new(
                ironclaw_loop_support::DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID,
            )
            .map_err(|error| {
                AgentLoopHostError::new(AgentLoopHostErrorKind::Internal, error.to_string())
            })?,
            output,
        });
        Ok(byte_len)
    }
}

fn workspace_mounts(permissions: MountPermissions) -> HarnessResult<MountView> {
    Ok(MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace")?,
        VirtualPath::new("/projects/workspace")?,
        permissions,
    )])?)
}

fn memory_mounts(permissions: MountPermissions) -> HarnessResult<MountView> {
    Ok(MountView::new(vec![MountGrant::new(
        MountAlias::new("/memory")?,
        VirtualPath::new("/memory")?,
        permissions,
    )])?)
}

fn skill_mounts() -> HarnessResult<MountView> {
    Ok(MountView::new(vec![
        MountGrant::new(
            MountAlias::new("/skills")?,
            VirtualPath::new("/projects/skills")?,
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/system/skills")?,
            VirtualPath::new("/projects/system/skills")?,
            MountPermissions::read_only(),
        ),
    ])?)
}

fn capability_grants(
    grantee: Principal,
    capabilities: &[CapabilityId],
    allowed_effects: Vec<EffectKind>,
    mounts: MountView,
    mount_overrides: &[(CapabilityId, MountView)],
    network: NetworkPolicy,
    secrets: Vec<SecretHandle>,
) -> CapabilitySet {
    CapabilitySet {
        grants: capabilities
            .iter()
            .map(|capability| {
                let mounts = mount_overrides
                    .iter()
                    .find(|(override_capability, _mounts)| override_capability == capability)
                    .map(|(_capability, mounts)| mounts.clone())
                    .unwrap_or_else(|| mounts.clone());
                CapabilityGrant {
                    id: CapabilityGrantId::new(),
                    capability: capability.clone(),
                    grantee: grantee.clone(),
                    issued_by: Principal::HostRuntime,
                    constraints: GrantConstraints {
                        allowed_effects: allowed_effects.clone(),
                        mounts,
                        network: network.clone(),
                        secrets: secrets.clone(),
                        resource_ceiling: None,
                        expires_at: None,
                        max_invocations: None,
                    },
                }
            })
            .collect(),
    }
}

fn host_runtime_harness_error(error: impl std::fmt::Display) -> AgentLoopHostError {
    AgentLoopHostError::new(AgentLoopHostErrorKind::InvalidInvocation, error.to_string())
}

#[derive(Clone)]
pub struct RecordingTestCapabilityPort {
    mode: CapabilityMode,
    expose_spawn_subagent: bool,
    use_subagent_allowed_tool: bool,
    invocations: Arc<Mutex<Vec<CapabilityInvocation>>>,
    next_result: Arc<AtomicUsize>,
    approval_calls: Arc<AtomicUsize>,
}

#[derive(Debug, Clone, Copy)]
enum CapabilityMode {
    Echo,
    ApprovalThenEcho,
    SpawnAuthThenApprovalThenEcho,
}

impl RecordingTestCapabilityPort {
    pub fn echo() -> Self {
        Self::new(CapabilityMode::Echo, false, false)
    }

    pub fn echo_with_spawn_subagent() -> Self {
        Self::new(CapabilityMode::Echo, true, false)
    }

    pub fn approval_then_echo() -> Self {
        Self::new(CapabilityMode::ApprovalThenEcho, false, false)
    }

    pub fn approval_then_echo_with_spawn_subagent() -> Self {
        Self::new(CapabilityMode::ApprovalThenEcho, true, false)
    }

    pub fn approval_then_allowed_tool_with_spawn_subagent() -> Self {
        Self::new(CapabilityMode::ApprovalThenEcho, true, true)
    }

    pub fn spawn_auth_then_approval_then_echo_with_spawn_subagent() -> Self {
        Self::new(CapabilityMode::SpawnAuthThenApprovalThenEcho, true, false)
    }

    pub fn spawn_auth_then_approval_then_allowed_tool_with_spawn_subagent() -> Self {
        Self::new(CapabilityMode::SpawnAuthThenApprovalThenEcho, true, true)
    }

    fn new(
        mode: CapabilityMode,
        expose_spawn_subagent: bool,
        use_subagent_allowed_tool: bool,
    ) -> Self {
        Self {
            mode,
            expose_spawn_subagent,
            use_subagent_allowed_tool,
            invocations: Arc::new(Mutex::new(Vec::new())),
            next_result: Arc::new(AtomicUsize::new(1)),
            approval_calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn primary_capability_id(&self) -> CapabilityId {
        let id = if self.use_subagent_allowed_tool {
            READ_FILE_CAPABILITY_ID
        } else {
            TEST_CAPABILITY_ID
        };
        CapabilityId::new(id).expect("valid capability id")
    }

    fn primary_tool_name(&self) -> &'static str {
        if self.use_subagent_allowed_tool {
            SUBAGENT_ALLOWED_TEST_TOOL_NAME
        } else {
            "test_echo"
        }
    }

    fn invocations(&self) -> Vec<CapabilityInvocation> {
        self.invocations.lock().unwrap().clone()
    }

    pub fn invocation_count(&self) -> usize {
        self.invocations.lock().unwrap().len()
    }

    fn capability_allowlist(&self) -> Vec<CapabilityId> {
        let mut allowlist = vec![self.primary_capability_id()];
        if self.expose_spawn_subagent {
            allowlist.push(
                CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID)
                    .expect("valid capability id"),
            );
        }
        allowlist
    }

    fn completed_result(&self) -> CapabilityOutcome {
        let ordinal = self.next_result.fetch_add(1, Ordering::SeqCst);
        CapabilityOutcome::Completed(CapabilityResultMessage {
            result_ref: ironclaw_turns::LoopResultRef::new(format!("result:test-echo-{ordinal}"))
                .expect("valid result ref"),
            safe_summary: "echo: hi".to_string(),
            progress: ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
            terminate_hint: false,
            byte_len: 0,
        })
    }
}

#[async_trait]
impl LoopCapabilityPort for RecordingTestCapabilityPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        let definitions = vec![ProviderToolDefinition {
            capability_id: self.primary_capability_id(),
            name: self.primary_tool_name().to_string(),
            description: "Echo a test payload".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "message": {"type": "string"}
                }
            }),
        }];
        Ok(definitions)
    }

    async fn register_provider_tool_call(
        &self,
        call: ProviderToolCall,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        let capability_id = self.primary_capability_id();
        Ok(CapabilityCallCandidate {
            surface_version: CapabilitySurfaceVersion::new(TEST_CAPABILITY_SURFACE_VERSION)
                .expect("valid surface version"),
            capability_id: capability_id.clone(),
            effective_capability_ids: vec![capability_id],
            input_ref: CapabilityInputRef::new(format!("input:{}", call.id))
                .expect("valid input ref"),
            provider_replay: Some(ProviderToolCallReplay {
                provider_id: call.provider_id,
                provider_model_id: call.provider_model_id,
                provider_turn_id: call.turn_id.unwrap_or_else(|| "trace-turn".to_string()),
                provider_call_id: call.id,
                provider_tool_name: call.name,
                arguments: call.arguments,
                response_reasoning: call.response_reasoning,
                reasoning: call.reasoning,
                signature: call.signature,
            }),
        })
    }

    async fn visible_capabilities(
        &self,
        _request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        let descriptors = vec![CapabilityDescriptorView {
            capability_id: self.primary_capability_id(),
            provider: Some(ExtensionId::new("test").expect("valid provider")),
            runtime: RuntimeKind::FirstParty,
            safe_name: self.primary_tool_name().to_string(),
            safe_description: "Echo a test payload".to_string(),
            concurrency_hint: ConcurrencyHint::SafeForParallel,
            parameters_schema: json!({"type": "object"}),
        }];
        Ok(VisibleCapabilitySurface {
            version: CapabilitySurfaceVersion::new(TEST_CAPABILITY_SURFACE_VERSION)
                .expect("valid surface version"),
            descriptors,
        })
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        self.invocations.lock().unwrap().push(request);
        if matches!(self.mode, CapabilityMode::ApprovalThenEcho)
            && self.approval_calls.fetch_add(1, Ordering::SeqCst) == 0
        {
            return Ok(CapabilityOutcome::ApprovalRequired {
                gate_ref: LoopGateRef::new("gate:test-approval").expect("valid gate ref"),
                safe_summary: "test approval required".to_string(),
                approval_resume: None,
            });
        }
        if matches!(self.mode, CapabilityMode::SpawnAuthThenApprovalThenEcho) {
            match self.approval_calls.fetch_add(1, Ordering::SeqCst) {
                0 => return Ok(self.completed_result()),
                1 => {
                    return Ok(CapabilityOutcome::ApprovalRequired {
                        gate_ref: LoopGateRef::new("gate:test-approval").expect("valid gate ref"),
                        safe_summary: "test approval required".to_string(),
                        approval_resume: None,
                    });
                }
                _ => {}
            }
        }
        Ok(self.completed_result())
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        let stop_on_first_suspension = request.stop_on_first_suspension;
        let mut outcomes = Vec::new();
        let mut stopped_on_suspension = false;
        for invocation in request.invocations {
            let outcome = self.invoke_capability(invocation).await?;
            let is_suspension = outcome.is_suspension();
            outcomes.push(outcome);
            if is_suspension && stop_on_first_suspension {
                stopped_on_suspension = true;
                break;
            }
        }
        Ok(CapabilityBatchOutcome {
            outcomes,
            stopped_on_suspension,
        })
    }
}

struct HarnessCapabilityPortFactory {
    port: Arc<RecordingTestCapabilityPort>,
}

#[async_trait]
impl LoopCapabilityPortFactory for HarnessCapabilityPortFactory {
    async fn create_capability_port(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        Ok(self.port.clone())
    }
}

struct StaticCapabilitySurfaceProfileResolver {
    allow_set: CapabilityAllowSet,
}

#[async_trait]
impl CapabilitySurfaceProfileResolver for StaticCapabilitySurfaceProfileResolver {
    async fn resolve(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<CapabilityAllowSet, CapabilityResolveError> {
        Ok(self.allow_set.clone())
    }
}

struct EmptyIdentityContextSource;

#[async_trait]
impl HostIdentityContextSource for EmptyIdentityContextSource {
    async fn load_identity_candidates(
        &self,
        _run_context: &LoopRunContext,
        _mode: PromptMode,
    ) -> Result<Vec<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
        Ok(Vec::new())
    }
}

fn product_scope() -> ResourceScope {
    test_product_scope("tenant-e2e", "host-user", "agent-e2e", Some("project-e2e"))
}

pub fn test_product_scope(
    tenant_id: &str,
    host_user_id: &str,
    agent_id: &str,
    project_id: Option<&str>,
) -> ResourceScope {
    resource_scope(
        TenantId::new(tenant_id).expect("valid tenant"),
        UserId::new(host_user_id).expect("valid user"),
        AgentId::new(agent_id).expect("valid agent"),
        project_id.map(|id| ProjectId::new(id).expect("valid project")),
    )
}

fn binding_request(
    ingress: &RebornTestIngress,
    conversation_id: &str,
) -> HarnessResult<ResolveBindingRequest> {
    binding_request_with_trigger(ingress, conversation_id, ProductTriggerReason::DirectChat)
}

fn binding_request_with_trigger(
    ingress: &RebornTestIngress,
    conversation_id: &str,
    trigger: ProductTriggerReason,
) -> HarnessResult<ResolveBindingRequest> {
    binding_request_with_trigger_and_actor(ingress, conversation_id, "alice", trigger)
}

fn binding_request_with_trigger_and_actor(
    ingress: &RebornTestIngress,
    conversation_id: &str,
    actor_id: &str,
    trigger: ProductTriggerReason,
) -> HarnessResult<ResolveBindingRequest> {
    let envelope = ingress.verified_text_envelope_with_trigger(
        "binding-probe",
        actor_id,
        conversation_id,
        "hi",
        trigger,
    )?;
    Ok(binding_request_from_envelope(&envelope))
}

fn binding_request_from_envelope(envelope: &ProductInboundEnvelope) -> ResolveBindingRequest {
    ResolveBindingRequest {
        adapter_id: envelope.adapter_id().clone(),
        installation_id: envelope.installation_id().clone(),
        external_actor_ref: envelope.external_actor_ref().clone(),
        external_conversation_ref: envelope.external_conversation_ref().clone(),
        external_event_id: envelope.external_event_id().clone(),
        route_kind: route_kind_for_envelope(envelope),
        auth_claim: envelope.auth_claim().clone(),
    }
}

fn thread_scope_from_binding(binding: &ResolvedBinding) -> HarnessResult<ThreadScope> {
    thread_scope_from_binding_with_route_kind(binding, ProductConversationRouteKind::Direct)
}

fn thread_scope_from_binding_with_route_kind(
    binding: &ResolvedBinding,
    _route_kind: ProductConversationRouteKind,
) -> HarnessResult<ThreadScope> {
    Ok(ThreadScope {
        tenant_id: binding.tenant_id.clone(),
        agent_id: binding
            .agent_id
            .clone()
            .ok_or("resolved binding missing agent id")?,
        project_id: binding.project_id.clone(),
        owner_user_id: binding.subject_user_id.clone(),
        mission_id: None,
    })
}

fn route_kind_for_envelope(envelope: &ProductInboundEnvelope) -> ProductConversationRouteKind {
    match envelope.payload() {
        ProductInboundPayload::UserMessage(message) => route_kind_for_trigger(message.trigger),
        ProductInboundPayload::Command(command) => route_kind_for_trigger(command.trigger),
        _ => ProductConversationRouteKind::Direct,
    }
}

fn route_kind_for_trigger(trigger: ProductTriggerReason) -> ProductConversationRouteKind {
    match trigger {
        ProductTriggerReason::DirectChat => ProductConversationRouteKind::Direct,
        ProductTriggerReason::BotMention
        | ProductTriggerReason::ReplyToBot
        | ProductTriggerReason::BotCommand
        | ProductTriggerReason::LinkedThreadAction => ProductConversationRouteKind::Shared,
    }
}

fn scoped_turns_fs<F>(
    backend: Arc<F>,
    binding: &ResolvedBinding,
) -> HarnessResult<Arc<ScopedFilesystem<F>>>
where
    F: RootFilesystem,
{
    let owner_user_id = binding
        .subject_user_id
        .as_ref()
        .unwrap_or(&binding.actor_user_id);
    let target = format!(
        "/engine/tenants/{}/users/{}/turns",
        binding.tenant_id, owner_user_id
    );
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/turns").expect("valid turns alias"),
        VirtualPath::new(target).expect("valid turns target"),
        MountPermissions::read_write_list_delete(),
    )])?;
    Ok(Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts)))
}

pub fn trace_tool_call_response() -> ironclaw_loop_support::HostManagedModelResponse {
    ironclaw_loop_support::HostManagedModelResponse {
        safe_text_deltas: Vec::new(),
        safe_reasoning_deltas: Vec::new(),
        usage: None,
        output: ParentLoopOutput::CapabilityCalls(vec![CapabilityCallCandidate {
            surface_version: CapabilitySurfaceVersion::new(TEST_CAPABILITY_SURFACE_VERSION)
                .expect("valid surface version"),
            capability_id: CapabilityId::new(TEST_CAPABILITY_ID).expect("valid capability id"),
            effective_capability_ids: vec![
                CapabilityId::new(TEST_CAPABILITY_ID).expect("valid capability id"),
            ],
            input_ref: CapabilityInputRef::new("input:trace-call-1").expect("valid input ref"),
            provider_replay: Some(ProviderToolCallReplay {
                provider_id: "trace_replay".to_string(),
                provider_model_id: "trace_replay".to_string(),
                provider_turn_id: "trace-turn".to_string(),
                provider_call_id: "call-1".to_string(),
                provider_tool_name: "test_echo".to_string(),
                arguments: json!({"message": "hi"}),
                response_reasoning: None,
                reasoning: None,
                signature: None,
            }),
        }]),
    }
}

pub fn assert_milestone_order(
    milestones: &[LoopHostMilestone],
    before: impl Fn(&LoopHostMilestoneKind) -> bool,
    after: impl Fn(&LoopHostMilestoneKind) -> bool,
) {
    let before_index = milestones
        .iter()
        .position(|milestone| before(&milestone.kind))
        .expect("before milestone should be present");
    let after_index = milestones
        .iter()
        .position(|milestone| after(&milestone.kind))
        .expect("after milestone should be present");
    assert!(
        before_index < after_index,
        "expected milestone order, got {:?}",
        milestones
            .iter()
            .map(|milestone| milestone.kind.kind_name())
            .collect::<Vec<_>>()
    );
}
