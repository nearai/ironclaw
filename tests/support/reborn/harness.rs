//! Reborn binary-E2E harness.
//!
//! This harness drives the product caller path used by the #3702 validation
//! ports:
//!
//! inbound bytes -> ProductAdapter -> DefaultProductWorkflow ->
//! DefaultInboundTurnService -> DefaultTurnCoordinator -> TurnRunScheduler ->
//! Reborn planned agent loop -> model/capability/transcript evidence.
//!
//! Documented test-support substitutions:
//! - the model gateway is scripted trace replay;
//! - the capability port is a local recording echo/approval port;
//! - external internet, delivery, and OAuth are not exercised by this harness.

#![allow(dead_code)] // Shared by staged Reborn binary-E2E validation ports.

// arch-exempt: large_file, Reborn binary-E2E + host-runtime capability harness; the
// mock-MCP scaffolding has been split into `harness_mcp.rs`, further focused splits
// (auth, hooks) are tracked in `tests/support/reborn/CLAUDE.md`.

use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use ironclaw_approvals::{ApprovalResolver, AutoApproveSettingInput, DenyApproval, LeaseApproval};
use ironclaw_auth::{
    AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountLabel, CredentialAccountStatus,
    CredentialOwnership, NewCredentialAccount, ProviderScope,
};
use ironclaw_authorization::{GrantAuthorizer, TrustAwareCapabilityDispatchAuthorizer};
use ironclaw_extensions::{ExtensionPackage, ExtensionRegistry};
use ironclaw_filesystem::{
    BackendCapabilities, BackendId, BackendKind, CompositeRootFilesystem, ContentKind,
    InMemoryBackend, IndexPolicy, LocalFilesystem, MountDescriptor, RootFilesystem,
    ScopedFilesystem, StorageClass,
};
use ironclaw_first_party_extensions::{WEB_GET_CONTENT_CAPABILITY_ID, WEB_SEARCH_CAPABILITY_ID};
use ironclaw_host_api::{
    Action, AgentId, ApprovalRequestId, CapabilityDescriptor, CapabilityGrant, CapabilityGrantId,
    CapabilityId, CapabilitySet, CredentialStageError, Decision, EffectKind, ExecutionContext,
    ExtensionId, GrantConstraints, HostPath, InvocationId, MountAlias, MountGrant,
    MountPermissions, MountView, NetworkPolicy, NetworkScheme, NetworkTargetPattern, Obligation,
    Obligations, PackageId, Principal, ProjectId, ProviderToolName, ResourceEstimate,
    ResourceScope, RuntimeCredentialAccountProviderId, RuntimeHttpEgress, RuntimeHttpEgressError,
    RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, RuntimeKind, SecretHandle, TenantId,
    ThreadId, TrustClass, UserId, VirtualPath,
};
use ironclaw_host_runtime::{
    APPLY_PATCH_CAPABILITY_ID, BUILTIN_FIRST_PARTY_PROVIDER, CancelRuntimeWorkOutcome,
    CancelRuntimeWorkRequest, CapabilitySurfacePolicy,
    CapabilitySurfaceVersion as HostRuntimeCapabilitySurfaceVersion, ECHO_CAPABILITY_ID,
    GLOB_CAPABILITY_ID, GREP_CAPABILITY_ID, HTTP_CAPABILITY_ID, HTTP_SAVE_CAPABILITY_ID,
    HostRuntime, HostRuntimeError, HostRuntimeHealth, HostRuntimeServices, HostRuntimeStatus,
    JSON_CAPABILITY_ID, LIST_DIR_CAPABILITY_ID, MEMORY_READ_CAPABILITY_ID,
    MEMORY_SEARCH_CAPABILITY_ID, MEMORY_TREE_CAPABILITY_ID, MEMORY_WRITE_CAPABILITY_ID,
    PROFILE_SET_CAPABILITY_ID, READ_FILE_CAPABILITY_ID, RuntimeCapabilityOutcome,
    RuntimeCapabilityRequest, RuntimeCapabilityResumeRequest, RuntimeCredentialAccessSecret,
    RuntimeCredentialAccountRequest, RuntimeCredentialAccountResolver, RuntimeProcessPort,
    RuntimeStatusRequest, SHELL_CAPABILITY_ID, SKILL_INSTALL_CAPABILITY_ID,
    SKILL_LIST_CAPABILITY_ID, SKILL_REMOVE_CAPABILITY_ID, SPAWN_SUBAGENT_CAPABILITY_ID,
    SurfaceKind, TIME_CAPABILITY_ID, TRACE_COMMONS_CREDITS_CAPABILITY_ID,
    TRACE_COMMONS_ONBOARD_CAPABILITY_ID, TRACE_COMMONS_PROFILE_SET_CAPABILITY_ID,
    TRACE_COMMONS_PROFILE_TOKEN_CAPABILITY_ID, TRACE_COMMONS_STATUS_CAPABILITY_ID,
    TRIGGER_CREATE_CAPABILITY_ID, TRIGGER_LIST_CAPABILITY_ID, TRIGGER_PAUSE_CAPABILITY_ID,
    TRIGGER_REMOVE_CAPABILITY_ID, TRIGGER_RESUME_CAPABILITY_ID,
    VisibleCapabilityRequest as RuntimeVisibleCapabilityRequest,
    VisibleCapabilitySurface as RuntimeVisibleCapabilitySurface, WRITE_FILE_CAPABILITY_ID,
    builtin_first_party_handlers, builtin_first_party_package,
};
use ironclaw_host_runtime::{SchedulerTurnRunWakeNotifier, TurnRunSchedulerHandle};
use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilityResolveError, CapabilityResultWrite,
    CapabilitySurfaceProfileResolver, CapabilityWriteResult, DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID,
    EmptyUserProfileSource, HostIdentityContextBuildError, HostIdentityContextCandidate,
    HostIdentityContextSource, HostManagedModelRequest, HostRuntimeLoopCapabilityPortFactory,
    JsonSpawnSubagentInputCodec, LoopCapabilityPortFactory, LoopCapabilityResultWriter,
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
    IdempotencyLedger, InboundTurnService, ProductConversationRouteKind, ProjectService,
    ResolveBindingRequest, ResolvedBinding,
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
};
use ironclaw_reborn_composition::test_support::SkillActivationTestSource;
use ironclaw_reborn_composition::{
    ProductLiveCapabilityIo, ProductLiveVisibleCapabilityRequestConfig, RebornBuildInput,
    RebornLocalDevApprovalTestParts, RebornProductAuthServices, build_reborn_services,
    visible_capability_request_for_run,
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

use super::{
    config::WaitConfig,
    extension_surface::{
        BUNDLED_EXTENSION_CAPABILITY_IDS, BUNDLED_EXTENSION_IDS, EXTENSION_LIFECYCLE_CAPABILITY_IDS,
    },
    filesystem::{BlockingTurnStatePutFilesystem, local_filesystem},
    github as github_support,
    harness_mcp::{
        build_loopback_mcp_runtime, local_dev_host_runtime_with_registry_egress_and_mcp,
        mcp_loopback_network_policy, mock_mcp_extension_package,
    },
    harness_web_access,
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
pub(crate) type HarnessCapabilityParts = (
    Arc<dyn LoopCapabilityPortFactory>,
    Arc<dyn CapabilitySurfaceProfileResolver>,
    Arc<dyn ironclaw_loop_support::LoopCapabilityInputResolver>,
    Arc<dyn LoopCapabilityResultWriter>,
    HarnessCapabilityRecorder,
);
pub(crate) type HarnessTurnStorageBackend = BlockingTurnStatePutFilesystem<InMemoryBackend>;
pub(crate) type HarnessTurnBackend = CompositeRootFilesystem;

pub struct RebornBinaryE2EHarness {
    ingress: RebornTestIngress,
    workflow: DefaultProductWorkflow,
    external_conversation_id: String,
    binding: ResolvedBinding,
    thread_scope: ThreadScope,
    turn_scope: TurnScope,
    turn_store: Arc<FilesystemTurnStateStore<HarnessTurnBackend>>,
    coordinator: Arc<dyn TurnCoordinator>,
    _product_harness: RebornProductWorkflowHarness,
    thread_harness: RebornThreadHarness,
    model_gateway: RebornTraceReplayModelGateway,
    capability_recorder: HarnessCapabilityRecorder,
    milestone_sink: Arc<ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink>,
    scheduler_handle: Option<TurnRunSchedulerHandle>,
    scheduler_notifier: Arc<SchedulerTurnRunWakeNotifier>,
    _turn_root: Arc<tempfile::TempDir>,
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
    thread_backend: Arc<InMemoryBackend>,
    turn_backend: Arc<HarnessTurnStorageBackend>,
    turn_root: Arc<tempfile::TempDir>,
}

impl RebornHarnessSharedStorage {
    pub fn new() -> HarnessResult<Self> {
        let product_root = Arc::new(tempfile::tempdir()?);
        let turn_root = Arc::new(tempfile::tempdir()?);
        Ok(Self {
            product_backend: Arc::new(local_filesystem(product_root.path())?),
            product_root,
            thread_backend: Arc::new(InMemoryBackend::new()),
            turn_backend: Arc::new(BlockingTurnStatePutFilesystem::new(InMemoryBackend::new())),
            turn_root,
        })
    }

    pub fn block_next_turn_state_put(&self) {
        self.turn_backend.block_next_put();
    }

    pub async fn wait_for_blocked_turn_state_put(&self) {
        self.turn_backend.wait_for_blocked_put().await;
    }

    pub fn release_blocked_turn_state_put(&self) {
        self.turn_backend.release_blocked_put();
    }
}

#[derive(Debug, Clone)]
pub struct RecordedCapabilityResult {
    pub capability_id: CapabilityId,
    pub output: serde_json::Value,
}

pub(crate) enum HarnessCapabilityMode {
    Recording(RecordingTestCapabilityPort),
    HostRuntime(Arc<HostRuntimeCapabilityHarness>),
}

#[derive(Clone)]
pub(crate) enum HarnessCapabilityRecorder {
    Recording(Arc<RecordingTestCapabilityPort>),
    HostRuntime(Arc<HostRuntimeCapabilityHarness>),
}

impl HarnessCapabilityRecorder {
    pub(crate) fn invocations(&self) -> Vec<CapabilityInvocation> {
        match self {
            Self::Recording(port) => port.invocations(),
            Self::HostRuntime(harness) => harness.invocations(),
        }
    }

    pub(crate) fn workspace_file_path(&self, relative: &str) -> Option<PathBuf> {
        match self {
            Self::Recording(_) => None,
            Self::HostRuntime(harness) => Some(harness.workspace_file_path(relative)),
        }
    }

    pub(crate) fn capability_results(&self) -> Vec<RecordedCapabilityResult> {
        match self {
            Self::Recording(_) => Vec::new(),
            Self::HostRuntime(harness) => harness.capability_results(),
        }
    }

    /// E-PROFILE: the local-dev memory filesystem backing the user-profile
    /// source for this backend, if any. `None` for the Echo backend and for
    /// HostRuntime harnesses without a profile filesystem.
    pub(crate) fn profile_filesystem(&self) -> Option<Arc<dyn RootFilesystem>> {
        match self {
            Self::Recording(_) => None,
            Self::HostRuntime(harness) => harness.profile_filesystem_for_test(),
        }
    }

    /// E-SKILL: the `HostSkillContextSource` to wire as the runtime's
    /// `skill_context_source` for this backend, if any. `None` for the Echo
    /// backend and for HostRuntime harnesses without skill activation.
    pub(crate) fn skill_context_source(
        &self,
    ) -> Option<Arc<dyn ironclaw_loop_support::HostSkillContextSource>> {
        match self {
            Self::Recording(_) => None,
            Self::HostRuntime(harness) => harness.skill_context_source_for_test(),
        }
    }

    /// C-ATTACH: the attachment read port + inbound lander for this backend, if
    /// any. `None` for the Echo backend and for HostRuntime harnesses without a
    /// local-dev workspace filesystem.
    pub(crate) fn attachment_test_support(
        &self,
    ) -> Option<ironclaw_reborn_composition::AttachmentTestSupport> {
        match self {
            Self::Recording(_) => None,
            Self::HostRuntime(harness) => harness.attachment_test_support_for_test(),
        }
    }

    pub(crate) fn runtime_http_requests(&self) -> Vec<RuntimeHttpEgressRequest> {
        match self {
            Self::Recording(_) => Vec::new(),
            Self::HostRuntime(harness) => harness.runtime_http_requests(),
        }
    }

    /// See [`HostRuntimeCapabilityHarness::process_commands`]; empty for the
    /// Echo recording backend.
    pub(crate) fn recorded_process_commands(&self) -> Vec<String> {
        match self {
            Self::Recording(_) => Vec::new(),
            Self::HostRuntime(harness) => harness.process_commands(),
        }
    }

    pub(crate) fn network_http_requests(&self) -> Vec<NetworkHttpRequest> {
        match self {
            Self::Recording(_) => Vec::new(),
            Self::HostRuntime(harness) => harness.network_http_requests(),
        }
    }

    pub(crate) async fn approve_local_dev_gate(&self, gate_ref: &GateRef) -> HarnessResult<()> {
        match self {
            Self::Recording(_) => {
                Err("recording capability port has no local-dev approvals".into())
            }
            Self::HostRuntime(harness) => harness.approve_local_dev_gate(gate_ref).await,
        }
    }

    pub(crate) async fn deny_local_dev_gate(&self, gate_ref: &GateRef) -> HarnessResult<()> {
        match self {
            Self::Recording(_) => {
                Err("recording capability port has no local-dev approvals".into())
            }
            Self::HostRuntime(harness) => harness.deny_local_dev_gate(gate_ref).await,
        }
    }

    pub(crate) async fn disable_auto_approve_for(&self, scope: ResourceScope) -> HarnessResult<()> {
        match self {
            Self::Recording(_) => {
                Err("recording capability port has no local-dev auto-approve settings".into())
            }
            Self::HostRuntime(harness) => harness.disable_global_auto_approve(scope).await,
        }
    }

    pub(crate) async fn enable_auto_approve_for(&self, scope: ResourceScope) -> HarnessResult<()> {
        match self {
            Self::Recording(_) => {
                Err("recording capability port has no local-dev auto-approve settings".into())
            }
            Self::HostRuntime(harness) => harness.enable_global_auto_approve(scope).await,
        }
    }

    pub(crate) fn approval_requests_store(
        &self,
    ) -> Option<Arc<dyn ironclaw_run_state::ApprovalRequestStore>> {
        match self {
            Self::Recording(_) => None,
            Self::HostRuntime(harness) => harness.approval_requests_store(),
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

    pub async fn with_model_gateway_scope_shared_storage(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_port: RecordingTestCapabilityPort,
        scope: ResourceScope,
        shared_storage: RebornHarnessSharedStorage,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway_scope_identity_source_trigger_installation_shared_storage(
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

    pub async fn with_model_gateway_scope_installation_shared_storage(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_port: RecordingTestCapabilityPort,
        scope: ResourceScope,
        adapter_id: &str,
        installation_id: &str,
        shared_storage: RebornHarnessSharedStorage,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway_scope_initial_actor_installation_shared_storage(
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
    pub async fn with_model_gateway_scope_initial_actor_installation_shared_storage(
        conversation_id: &str,
        initial_actor_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_port: RecordingTestCapabilityPort,
        scope: ResourceScope,
        adapter_id: &str,
        installation_id: &str,
        shared_storage: RebornHarnessSharedStorage,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway_scope_identity_source_trigger_installation_shared_storage(
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
    pub async fn with_model_gateway_scope_identity_source_trigger_installation_shared_storage(
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
        Self::with_model_gateway_capability_mode_identity_source_trigger_storage_and_adapter(
            conversation_id,
            model_gateway,
            HarnessCapabilityMode::Recording(capability_port),
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

    pub async fn with_model_gateway_identity_source_shared(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_port: RecordingTestCapabilityPort,
        identity_context_source: Arc<dyn HostIdentityContextSource>,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway_options_identity_source_trigger(
            conversation_id,
            model_gateway,
            capability_port,
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

    pub async fn with_host_runtime_qa_smoke_capabilities(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
    ) -> HarnessResult<Self> {
        let host_runtime = Arc::new(HostRuntimeCapabilityHarness::qa_smoke_tools().await?);
        Self::with_model_gateway_capability_mode(
            conversation_id,
            model_gateway,
            HarnessCapabilityMode::HostRuntime(host_runtime),
            false,
        )
        .await
    }

    pub async fn with_host_runtime_extension_lifecycle_capabilities(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
    ) -> HarnessResult<Self> {
        let host_runtime =
            Arc::new(HostRuntimeCapabilityHarness::extension_lifecycle_tools().await?);
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

    pub async fn with_host_runtime_trace_commons_capabilities(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
    ) -> HarnessResult<Self> {
        let host_runtime = Arc::new(HostRuntimeCapabilityHarness::trace_commons_tools().await?);
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
        Self::with_model_gateway_options_identity_source(
            conversation_id,
            model_gateway,
            capability_port,
            accept_harness_blocked_evidence,
            Arc::new(EmptyIdentityContextSource),
        )
        .await
    }

    async fn with_model_gateway_options_identity_source(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_port: RecordingTestCapabilityPort,
        accept_harness_blocked_evidence: bool,
        identity_context_source: Arc<dyn HostIdentityContextSource>,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway_options_identity_source_trigger(
            conversation_id,
            model_gateway,
            capability_port,
            accept_harness_blocked_evidence,
            ProductTriggerReason::DirectChat,
            identity_context_source,
        )
        .await
    }

    async fn with_model_gateway_options_identity_source_trigger(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_port: RecordingTestCapabilityPort,
        accept_harness_blocked_evidence: bool,
        initial_trigger: ProductTriggerReason,
        identity_context_source: Arc<dyn HostIdentityContextSource>,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway_capability_mode_identity_source_trigger(
            conversation_id,
            model_gateway,
            HarnessCapabilityMode::Recording(capability_port),
            accept_harness_blocked_evidence,
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
        Self::with_model_gateway_capability_mode_identity_source(
            conversation_id,
            model_gateway,
            capability_mode,
            accept_harness_blocked_evidence,
            Arc::new(EmptyIdentityContextSource),
        )
        .await
    }

    async fn with_model_gateway_capability_mode_identity_source(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_mode: HarnessCapabilityMode,
        accept_harness_blocked_evidence: bool,
        identity_context_source: Arc<dyn HostIdentityContextSource>,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway_capability_mode_identity_source_trigger(
            conversation_id,
            model_gateway,
            capability_mode,
            accept_harness_blocked_evidence,
            ProductTriggerReason::DirectChat,
            identity_context_source,
        )
        .await
    }

    async fn with_model_gateway_capability_mode_identity_source_trigger(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_mode: HarnessCapabilityMode,
        accept_harness_blocked_evidence: bool,
        initial_trigger: ProductTriggerReason,
        identity_context_source: Arc<dyn HostIdentityContextSource>,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway_capability_mode_identity_source_trigger_storage_and_adapter(
            conversation_id,
            model_gateway,
            capability_mode,
            accept_harness_blocked_evidence,
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
    async fn with_model_gateway_capability_mode_identity_source_trigger_storage_and_adapter(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_mode: HarnessCapabilityMode,
        accept_harness_blocked_evidence: bool,
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
            (
                Arc::new(BlockingTurnStatePutFilesystem::new(InMemoryBackend::new())),
                turn_root,
            )
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
                // Keep the durable runner heartbeat at its production default;
                // test responsiveness comes from fast scheduler polling below.
                poll_interval: Duration::from_millis(10),
                ..DefaultPlannedRuntimeConfig::default()
            },
            model_route_resolver: None,
            cancellation_factory: None,
            skill_context_source: None,
            input_queue: None,
            identity_context_source,
            user_profile_source: Arc::new(EmptyUserProfileSource),
            model_policy_guard: None,
            model_budget_accountant: None,
            safety_context: None,
            hook_dispatcher_builder_factory: None,
            communication_context_provider: None,
            hook_security_audit_sink: None,
            turn_event_sink: None,
            attachment_read_port: None,
            scheduler_wake_wiring: None,
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
        turn_store: Arc<FilesystemTurnStateStore<HarnessTurnBackend>>,
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
        let scheduler_notifier = composition.scheduler_handle.wake_notifier();
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
            scheduler_handle: Some(composition.scheduler_handle),
            scheduler_notifier,
            _turn_root: turn_root,
        }
    }

    pub fn start(&mut self) {
        // The scheduler is started automatically inside build_default_planned_runtime.
        // This method is kept for API compatibility.
    }

    pub fn start_workers(&mut self, _count: usize) {
        // The scheduler is started automatically inside build_default_planned_runtime.
        // Worker count is configured via DefaultPlannedRuntimeConfig.worker_count.
    }

    pub async fn shutdown(&mut self) {
        if let Some(scheduler) = self.scheduler_handle.take() {
            scheduler.shutdown().await;
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
                resume_disposition: None,
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
            // A terminal status (Completed/Failed/Cancelled/RecoveryRequired) is
            // never left, so once we observe one that is not the target the run
            // can never reach `expected`. Fail fast instead of polling to the
            // deadline — otherwise a run that fails early (e.g. a spawn capability
            // returning a terminal `driver_unavailable`) burns the whole timeout
            // and buries the real failure category.
            if state.status.is_terminal() {
                return Err(format!(
                    "expected {expected:?} but run reached terminal status {:?}; failure={:?}",
                    state.status, state.failure
                )
                .into());
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
        // Scheduler handle is Option<TurnRunSchedulerHandle>; shutdown is async
        // and cannot be called from Drop. The handle is taken in shutdown() and
        // here we just let it drop. The scheduler supervisor task exits when the
        // command channel closes on drop.
        let _ = self.scheduler_handle.take();
    }
}

struct HarnessLoopExitEvidencePort {
    inner: ThreadCheckpointLoopExitEvidencePort<FilesystemSessionThreadService<InMemoryBackend>>,
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
    pub(crate) fn into_parts(
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

/// Backing handles for the two synthetic `outbound_delivery_*` capabilities
/// (C-SYNTH outbound seam). `Some` only for `outbound_target_tools()`. Bundles
/// the injected facade double + the settings stores the production
/// `outbound_delivery_capabilities` wiring consumes, so the harness struct
/// widens by ONE field instead of four. The auto-approve store and
/// approval-request/lease stores are already held as sibling harness fields
/// (`auto_approve_settings` / `approval_parts`) and re-used, not duplicated here.
struct OutboundTargetToolsParts {
    /// Concrete double (not the trait object) so tests can read `set` calls back;
    /// upcast to `Arc<dyn OutboundPreferencesProductFacade>` at wrap time.
    facade: Arc<super::outbound_preferences::FakeOutboundPreferencesFacade>,
    requires_approval: bool,
    tool_permission_overrides: Arc<dyn ironclaw_approvals::ToolPermissionOverrideStore>,
    persistent_approval_policies: Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStore>,
}

pub(crate) struct HostRuntimeCapabilityHarness {
    runtime: Arc<dyn HostRuntime>,
    approval_parts: Option<RebornLocalDevApprovalTestParts>,
    auto_approve_settings: Option<Arc<dyn ironclaw_approvals::AutoApproveSettingStore>>,
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
    additional_provider_trust: Vec<(ExtensionId, Vec<EffectKind>)>,
    user_id: UserId,
    invocations: Arc<Mutex<Vec<CapabilityInvocation>>>,
    results: Arc<Mutex<Vec<RecordedCapabilityResult>>>,
    http_egress: Option<Arc<RecordingRuntimeHttpEgress>>,
    network_egress: Option<Arc<RecordingNetworkHttpEgress>>,
    /// Inert recording process port (slice 5). `Some` when the harness injected
    /// a `RecordingProcessPort`; `None` when the live `LocalHostProcessPort` was
    /// used (`.with_live_shell()` path) or the harness predates slice 5.
    process_port: Option<Arc<super::process::RecordingProcessPort>>,
    /// Raw local-dev memory filesystem backing the user-profile source
    /// (E-PROFILE seam). `Some` only for `new_with_options`-built harnesses (which
    /// flow through `RebornServices`); `None` for the lower-level constructors and
    /// the Echo backend. Read back via `profile_filesystem_for_test`.
    profile_filesystem: Option<Arc<dyn RootFilesystem>>,
    /// Project service for the local-dev synthetic `project_create` capability
    /// (E-PROJ seam). `Some` for every `new_with_options`-built local-dev harness;
    /// `None` for the lower-level constructors and the Echo backend.
    /// `create_capability_port` wraps the port with the synthetic project-create
    /// capability ONLY when `PROJECT_CREATE_CAPABILITY_ID` is also in
    /// `capability_ids` (i.e. only `project_tools()` surfaces it), so other
    /// local-dev groups are unaffected. Tests read projects back via `project_service`.
    project_service: Option<Arc<dyn ProjectService>>,
    /// Local-dev skill context source for the synthetic `skill_activate`
    /// capability and runtime prompt injection (E-SKILL seam). `Some` only for
    /// `skill_activation_tools()`; `None` otherwise. `create_capability_port`
    /// wraps the port with the synthetic `skill_activate` capability ONLY when
    /// `SKILL_ACTIVATE_CAPABILITY_ID` is also in `capability_ids`, and its
    /// `context_source()` is wired as the runtime's `skill_context_source` in
    /// `into_group`. Held as the opaque test-support handle so this crate never
    /// names the crate-private source type.
    skill_activation_source: Option<SkillActivationTestSource>,
    /// Attachment read port + inbound lander backing the C-ATTACH seam. `Some`
    /// only for `new_with_options`-built harnesses (which flow through
    /// `RebornServices`, and thus have a local-dev workspace filesystem to build
    /// both over); `None` for the lower-level constructors and the Echo backend.
    /// Read back via `attachment_test_support_for_test`.
    attachment_test_support: Option<ironclaw_reborn_composition::AttachmentTestSupport>,
    /// Backing handles for the synthetic `outbound_delivery_*` capabilities
    /// (C-SYNTH outbound seam). `Some` only for `outbound_target_tools()`;
    /// `create_capability_port` wraps the port with the two capabilities via
    /// `apply_synthetic_capability_wrappers` when this is `Some`.
    outbound_target_tools: Option<OutboundTargetToolsParts>,
    /// C-MULTIUSER seam: when `true`, [`create_capability_port`] resolves the
    /// capability-execution user from the RUN's owner/actor (mirroring
    /// production `local_dev_visible_capability_request`,
    /// `crates/ironclaw_reborn_composition/src/runtime/local_dev.rs`) instead of
    /// this harness's single fixed `user_id`. That is what lets two distinct
    /// actors dispatching over the group's ONE shared capability backend run
    /// under DISTINCT `(tenant, user)` scopes, so memory, auto-approve, and
    /// approval-settings isolate per actor — the real production behavior.
    /// Defaults `false` so every existing fixed-user harness is byte-identical;
    /// only the multiuser group constructors flip it on via
    /// [`with_run_owner_scoped_capability_dispatch`].
    scope_capability_by_run_owner: bool,
    /// Local-dev product-auth services (C-JOURNEY convergence seam). `Some`
    /// only for `new_with_options`-built harnesses (which flow through
    /// `RebornServices`); `None` for the lower-level constructors and the Echo
    /// backend. `seed_github_credential_account` reads this to create a real
    /// credential account through `credential_account_service()`, letting a
    /// parked `github.*` auth gate's `ProductAuthRuntimeCredentialResolver`
    /// lookup resolve on re-dispatch.
    product_auth: Option<Arc<RebornProductAuthServices>>,
}

struct HostRuntimeHarnessOptions {
    mounts: MountView,
    runtime_policy: Option<ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy>,
    seed_extension_credentials: bool,
    /// Tenant the E-SKILL skill context source is constructed under, when this
    /// harness surfaces the synthetic `skill_activate` capability. Only
    /// `skill_activation_tools()` sets this (via
    /// `with_skill_activation_tenant`), passing the SAME tenant the caller's
    /// group run scope resolved (`group.rs` `build_base`'s
    /// `canonical_binding.tenant_id`) — never a separately hardcoded literal —
    /// so `skill_activate` resolves the seeded user skill against the same
    /// tenant the turn runs under. `None` for every other harness variant.
    skill_activation_tenant: Option<TenantId>,
    /// Injected outbound-delivery facade double + `target_set` approval flag,
    /// when this harness surfaces the synthetic `outbound_delivery_*`
    /// capabilities (C-SYNTH outbound seam). Only `outbound_target_tools()` sets
    /// this. `new_with_options` pairs the facade with the local-dev settings
    /// stores captured from `RebornServices` to build `OutboundTargetToolsParts`.
    outbound_target_facade: Option<(
        Arc<super::outbound_preferences::FakeOutboundPreferencesFacade>,
        bool,
    )>,
    /// C-JOURNEY: override the local-dev host network HTTP egress
    /// (`RebornBuildInput::with_network_http_egress_for_test`). Without this,
    /// `build_local_runtime` defaults to a REAL `ReqwestNetworkTransport`
    /// (`factory.rs`), so any harness dispatching a bundled WASM capability
    /// that crosses HTTP (e.g. `github.*`) on the `new_with_options` path MUST
    /// set this to stay hermetic. `None` for every harness that surfaces no
    /// such capability.
    network_http_egress_for_test: Option<Arc<dyn NetworkHttpEgress>>,
    /// C-JOURNEY: bundled first-party WASM packages (e.g. github) to publish
    /// directly into the local-dev active-extension registry at construction
    /// time, via `RebornServices::publish_bundled_extension_for_test`
    /// (reaches the SAME `ActiveExtensionPublisher::publish` step
    /// `builtin.extension_activate` calls). Without this, a bundled package's
    /// capabilities are granted/trusted at the harness-authority layer
    /// (`capability_ids`/`additional_provider_trust`) but NOT present in the
    /// runtime's own dispatchable registry, so dispatch silently no-ops (the
    /// tool call never reaches `invoke_capability`). Empty for every harness
    /// that surfaces no bundled WASM capability.
    activate_bundled_extensions_for_test: Vec<ExtensionPackage>,
}

impl HostRuntimeHarnessOptions {
    fn new(
        mounts: MountView,
        runtime_policy: Option<ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy>,
    ) -> Self {
        Self {
            mounts,
            runtime_policy,
            seed_extension_credentials: false,
            skill_activation_tenant: None,
            outbound_target_facade: None,
            network_http_egress_for_test: None,
            activate_bundled_extensions_for_test: Vec::new(),
        }
    }

    fn with_seed_extension_credentials(mut self) -> Self {
        self.seed_extension_credentials = true;
        self
    }

    fn with_skill_activation_tenant(mut self, tenant: TenantId) -> Self {
        self.skill_activation_tenant = Some(tenant);
        self
    }

    fn with_outbound_target_tools(
        mut self,
        facade: Arc<super::outbound_preferences::FakeOutboundPreferencesFacade>,
        target_set_requires_approval: bool,
    ) -> Self {
        self.outbound_target_facade = Some((facade, target_set_requires_approval));
        self
    }

    fn with_network_http_egress_for_test(mut self, egress: Arc<dyn NetworkHttpEgress>) -> Self {
        self.network_http_egress_for_test = Some(egress);
        self
    }

    fn with_activated_bundled_extension(mut self, package: ExtensionPackage) -> Self {
        self.activate_bundled_extensions_for_test.push(package);
        self
    }
}

impl HostRuntimeCapabilityHarness {
    async fn file_tools() -> HarnessResult<Self> {
        let harness = Self::file_tools_with_runtime_policy(Some(
            ironclaw_reborn_composition::local_dev_yolo_runtime_policy(true)?,
        ))
        .await?;
        harness
            .enable_global_auto_approve_for_product_and_harness_users()
            .await?;
        Ok(harness)
    }

    pub(crate) async fn file_tools_requiring_approval() -> HarnessResult<Self> {
        let harness = Self::file_tools_with_runtime_policy(None).await?;
        // Global auto-approve now defaults ON, so disable it explicitly to keep
        // this constructor's per-tool approval gate behavior.
        harness
            .disable_global_auto_approve_for_product_and_harness_users()
            .await?;
        Ok(harness)
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
        let harness = Self::new(
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
        .await?;
        harness
            .enable_global_auto_approve_for_product_and_harness_users()
            .await?;
        Ok(harness)
    }

    async fn process_tools() -> HarnessResult<Self> {
        let harness = Self::new_with_options(
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
        .await?;
        harness
            .enable_global_auto_approve_for_product_and_harness_users()
            .await?;
        Ok(harness)
    }

    async fn qa_smoke_tools() -> HarnessResult<Self> {
        let (root, storage_root, workspace_root) = host_runtime_storage_roots()?;
        std::fs::create_dir_all(storage_root.join("skills"))?;
        std::fs::create_dir_all(storage_root.join("system/skills"))?;
        let runtime = local_dev_host_runtime_with_http_egress(
            storage_root,
            Arc::new(RecordingRuntimeHttpEgress::with_body(
                br#"{"accepted":true,"source":"qa-smoke"}"#.to_vec(),
            )),
            // qa_smoke_tools exercises real process execution (SpawnProcess effect);
            // leave the default LocalHostProcessPort in place.
            None,
        )?;
        let mounts = qa_smoke_mounts()?;
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
            auto_approve_settings: None,
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
                CapabilityId::new(ECHO_CAPABILITY_ID)?,
                CapabilityId::new(TIME_CAPABILITY_ID)?,
                CapabilityId::new(JSON_CAPABILITY_ID)?,
                CapabilityId::new(HTTP_CAPABILITY_ID)?,
                CapabilityId::new(HTTP_SAVE_CAPABILITY_ID)?,
                CapabilityId::new(MEMORY_SEARCH_CAPABILITY_ID)?,
                CapabilityId::new(MEMORY_WRITE_CAPABILITY_ID)?,
                CapabilityId::new(MEMORY_READ_CAPABILITY_ID)?,
                CapabilityId::new(MEMORY_TREE_CAPABILITY_ID)?,
                CapabilityId::new(READ_FILE_CAPABILITY_ID)?,
                CapabilityId::new(WRITE_FILE_CAPABILITY_ID)?,
                CapabilityId::new(LIST_DIR_CAPABILITY_ID)?,
                CapabilityId::new(GLOB_CAPABILITY_ID)?,
                CapabilityId::new(GREP_CAPABILITY_ID)?,
                CapabilityId::new(APPLY_PATCH_CAPABILITY_ID)?,
                CapabilityId::new(SHELL_CAPABILITY_ID)?,
                CapabilityId::new(SPAWN_SUBAGENT_CAPABILITY_ID)?,
                CapabilityId::new(SKILL_LIST_CAPABILITY_ID)?,
                CapabilityId::new(SKILL_INSTALL_CAPABILITY_ID)?,
                CapabilityId::new(SKILL_REMOVE_CAPABILITY_ID)?,
                CapabilityId::new(TRIGGER_CREATE_CAPABILITY_ID)?,
                CapabilityId::new(TRIGGER_LIST_CAPABILITY_ID)?,
                CapabilityId::new(TRIGGER_PAUSE_CAPABILITY_ID)?,
                CapabilityId::new(TRIGGER_RESUME_CAPABILITY_ID)?,
                CapabilityId::new(TRIGGER_REMOVE_CAPABILITY_ID)?,
            ],
            runtime_kind: RuntimeKind::FirstParty,
            effect_kinds: vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::DeleteFilesystem,
                EffectKind::Network,
                EffectKind::SpawnProcess,
                EffectKind::ExecuteCode,
                EffectKind::ExternalWrite,
            ],
            network_policy: http_test_policy(),
            secrets: Vec::new(),
            provider_id: ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            additional_provider_trust: Vec::new(),
            user_id: UserId::new("reborn-e2e-qa-smoke-user")?,
            invocations: Arc::new(Mutex::new(Vec::new())),
            results: Arc::new(Mutex::new(Vec::new())),
            http_egress: None,
            network_egress: None,
            process_port: None,
            profile_filesystem: None,
            project_service: None,
            skill_activation_source: None,
            attachment_test_support: None,
            outbound_target_tools: None,
            scope_capability_by_run_owner: false,
            product_auth: None,
        })
    }

    pub(crate) async fn extension_lifecycle_tools() -> HarnessResult<Self> {
        let mut capability_ids = capability_ids_from_strs(EXTENSION_LIFECYCLE_CAPABILITY_IDS)?;
        capability_ids.extend(capability_ids_from_strs(BUNDLED_EXTENSION_CAPABILITY_IDS)?);
        let mut harness = Self::new_with_options(
            "reborn-e2e-extension-lifecycle-tools",
            capability_ids,
            local_dev_all_effects(),
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-extension-lifecycle-user")?,
            HostRuntimeHarnessOptions::new(
                MountView::default(),
                Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                    true,
                )?),
            )
            .with_seed_extension_credentials(),
        )
        .await?;
        harness.network_policy = wildcard_test_policy();
        harness.additional_provider_trust = bundled_extension_provider_trust()?;
        harness
            .enable_global_auto_approve_for_product_and_harness_users()
            .await?;
        Ok(harness)
    }

    /// C-JOURNEY convergence seam: surfaces the file-tool approval-gate
    /// capabilities (`write_file`/`read_file`, `PermissionMode::Ask` — same
    /// grant shape as `file_tools_requiring_approval`) AND a single GitHub
    /// capability (`github.get_repo`) on the SAME `build_reborn_services`
    /// local-dev runtime — the one wired with the
    /// `run_state`/`approval_requests`/`capability_leases` stores BOTH gate
    /// classes' resume paths need (`new_with_options` -> `build_reborn_services`).
    ///
    /// Distinct from `github_issue_tools_auth_required` (a separate,
    /// lower-level `HostRuntimeServices` build with a hardcoded
    /// `FixedRuntimeCredentialAccountResolver` and no run_state store — see
    /// that constructor's doc comment): this harness's `github.*` credential
    /// resolves through the REAL `ProductAuthRuntimeCredentialResolver`
    /// (`factory.rs`, wired unconditionally by `build_reborn_services`). No
    /// GitHub credential account is seeded at construction (unlike
    /// `extension_lifecycle_tools`, which seeds all four bundled providers via
    /// `.with_seed_extension_credentials()`).
    ///
    /// **Gate chaining (empirically verified, not assumed):** the global
    /// auto-approve toggle this harness disables (for the file-tool arm) is
    /// NOT capability-scoped, so `github.get_repo` first raises a real
    /// `TurnStatus::BlockedApproval` too. Approving re-dispatches the
    /// still-uncredentialed capability, which blocks AGAIN at a real
    /// `TurnStatus::BlockedAuth` (`CredentialStageError::AuthRequired`).
    /// `RebornIntegrationHarness::resolve_auth_gate` seeds the account
    /// (`seed_github_credential_account`) and resumes, letting the SAME
    /// parked capability re-dispatch and complete — the happy-path auth
    /// resume the `github_issue_tools_auth_required` fixture cannot do. See
    /// `scenario_auth_then_approval_journey`'s module doc for the full
    /// approval->auth chain a caller must drive.
    ///
    /// **Making `github.*` genuinely dispatchable (not just granted) needed
    /// two additive test-support seams, both required together:**
    /// 1. `capability_ids`/`additional_provider_trust` alone are NOT enough —
    ///    they only populate the harness-authority grant layer. The runtime's
    ///    OWN dispatchable registry (`build_local_runtime`'s
    ///    `local_dev_builtin_extension_registry()`) contains only first-party
    ///    builtins + the four lifecycle capabilities; bundled packages
    ///    (github, gmail, …) live in a SEPARATE `AvailableExtensionCatalog`
    ///    used for search only. Without registry presence, a scripted
    ///    `github.*` call silently never reaches `invoke_capability` (the run
    ///    completes with zero recorded invocations). Fixed via
    ///    `RebornServices::publish_bundled_extension_for_test`
    ///    (`factory.rs`, new `#[cfg(feature = "test-support")]` accessor) —
    ///    reaches the SAME `ActiveExtensionPublisher::publish` step
    ///    `builtin.extension_activate` calls, called directly at harness
    ///    construction instead of via a scripted install/activate handshake.
    /// 2. Registry presence alone still isn't sufficient: `build_local_runtime`
    ///    mounts `/system/extensions` at an EMPTY per-harness tempdir, so the
    ///    runtime fails to compile `wasm/github_tool.wasm` at dispatch time
    ///    (`Failed{host_creation_failed}`) even once the package metadata is
    ///    registered. Fixed by copying the REAL asset directory
    ///    (`github_support::asset_root()`, already used by the
    ///    `github_issue_tools_*` harnesses) into this harness's own tempdir
    ///    mount (`copy_dir_recursive`) — no new fixtures, reuses the existing
    ///    on-disk asset tree.
    ///
    /// Runtime policy is left at `None` (like `file_tools_requiring_approval`,
    /// NOT the `LocalDevYolo` policy `extension_lifecycle_tools` uses) so the
    /// file tools' real `PermissionMode::Ask` gate is preserved; the two seams
    /// above are independent of the runtime-policy profile.
    pub(crate) async fn file_and_github_auth_tools() -> HarnessResult<Self> {
        // Hermetic guard: `new_with_options`'s `build_local_runtime` defaults to
        // a REAL `ReqwestNetworkTransport` when no test egress is supplied
        // (`factory.rs`). This harness surfaces a `github.*` WASM capability
        // that crosses HTTP, so it MUST override the network egress or the
        // post-resume dispatch would attempt a live network call.
        let github_fixture_response =
            br#"{"id":1,"full_name":"octocat/hello-world","private":false}"#.to_vec();
        let network_egress: Arc<dyn NetworkHttpEgress> = Arc::new(
            RecordingNetworkHttpEgress::with_body(github_fixture_response),
        );
        let mut harness = Self::new_with_options(
            "reborn-e2e-file-github-auth-tools",
            vec![
                CapabilityId::new(WRITE_FILE_CAPABILITY_ID)?,
                CapabilityId::new(READ_FILE_CAPABILITY_ID)?,
                CapabilityId::new("github.get_repo")?,
            ],
            local_dev_all_effects(),
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-file-github-auth-user")?,
            HostRuntimeHarnessOptions::new(
                workspace_mounts(MountPermissions::read_write_list_delete())?,
                None,
            )
            .with_network_http_egress_for_test(network_egress)
            .with_activated_bundled_extension(github_support::extension_package()?),
        )
        .await?;
        harness.network_policy = wildcard_test_policy();
        harness.additional_provider_trust = bundled_extension_provider_trust()?;
        // See point 2 of this constructor's doc comment: registry presence
        // alone isn't enough, the WASM asset bytes must be copied into this
        // harness's own tempdir mount too.
        copy_dir_recursive(
            &github_support::asset_root(),
            &harness
                .root
                .path()
                .join("local-dev/system/extensions/github"),
        )?;
        // Global auto-approve now defaults ON; disable it so write_file/read_file
        // raise real `BlockedApproval` gates (mirrors `file_tools_requiring_approval`).
        // The GitHub auth gate is a separate mechanism (credential resolution, not
        // approval mode) and is unaffected by this toggle.
        harness
            .disable_global_auto_approve_for_product_and_harness_users()
            .await?;
        Ok(harness)
    }

    /// C-JOURNEY: seed a real GitHub credential account — WITH real secret
    /// material — through the PRODUCTION manual-token flow
    /// (`request_manual_token_setup` → `submit_manual_token`, the same
    /// two-step path the real "user pastes a token in the UI" flow drives), so
    /// a parked `github.*` auth gate's `ProductAuthRuntimeCredentialResolver`
    /// lookup resolves on re-dispatch AND the re-dispatched WASM capability's
    /// credential obligation can actually stage the token. A bare
    /// `credential_account_service().create_account(..)` carrying a dangling
    /// `SecretHandle` is NOT enough: it clears the auth gate, but the
    /// re-dispatched execution then fails at `stage_credential_material`
    /// (no material behind the handle) and the run still completes with a
    /// model-visible failure — caught by `assert_tool_result_contains`.
    ///
    /// `scope` MUST be the run's actual dispatch-time `(tenant, user, agent,
    /// project)` — `ProductAuthRuntimeCredentialResolver::resolve_access_secret`'s
    /// `account_visible_from_runtime_scope` check matches on all four, so a
    /// mismatched scope (e.g. a fixed literal that doesn't match the calling
    /// group/harness's real run scope) silently seeds an account the
    /// dispatch-time lookup never finds, leaving the run stuck at
    /// `BlockedAuth`. Callers build this from their own resolved run scope
    /// (see `RebornIntegrationHarness::resolve_auth_gate`, which uses
    /// `self.turn_scope` + `self.binding.actor_user_id` — the SAME fields
    /// `resume_run` uses).
    ///
    /// Continuation is `AuthContinuationRef::SetupOnly`: the harness's
    /// `resolve_auth_gate` performs the run resume itself (mirroring the
    /// approval-gate helpers), so the flow must not ALSO dispatch a
    /// `TurnGateResume` continuation.
    pub(crate) async fn seed_github_credential_account(
        &self,
        scope: &ResourceScope,
    ) -> HarnessResult<()> {
        let product_auth = self
            .product_auth
            .as_ref()
            .ok_or("harness missing local-dev product auth (not built via new_with_options)")?;
        let scope = AuthProductScope::credential_owner(scope, AuthSurface::Api);
        let challenge = product_auth
            .request_manual_token_setup(
                ironclaw_reborn_composition::RebornManualTokenSetupRequest::new(
                    scope.clone(),
                    AuthProviderId::new("github")?,
                    CredentialAccountLabel::new("journey github")?,
                    ironclaw_auth::AuthContinuationRef::SetupOnly,
                    chrono::Utc::now() + chrono::Duration::minutes(10),
                ),
            )
            .await
            .map_err(|error| format!("manual token setup failed: {error:?}"))?;
        product_auth
            .submit_manual_token(
                ironclaw_reborn_composition::RebornManualTokenSubmitRequest::new(
                    scope.clone(),
                    challenge.interaction_id,
                    secrecy::SecretString::from("journey-github-token"),
                ),
            )
            .await
            .map_err(|error| format!("manual token submit failed: {error:?}"))?;
        Ok(())
    }

    /// E-PROJ: harness surfacing the local-dev synthetic `project_create`
    /// capability. `create_capability_port` injects the synthetic capability via
    /// `apply_synthetic_capability_wrappers` because `PROJECT_CREATE_CAPABILITY_ID`
    /// is in the allowlist. Auto-approve is enabled so the capability dispatches
    /// without a gate.
    pub(crate) async fn project_tools() -> HarnessResult<Self> {
        let harness = Self::new_with_options(
            "reborn-e2e-project-tools",
            vec![CapabilityId::new(
                ironclaw_reborn_composition::test_support::PROJECT_CREATE_CAPABILITY_ID,
            )?],
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
            ],
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-project-tools-user")?,
            HostRuntimeHarnessOptions::new(
                MountView::default(),
                Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                    true,
                )?),
            ),
        )
        .await?;
        harness
            .enable_global_auto_approve_for_product_and_harness_users()
            .await?;
        Ok(harness)
    }

    /// C-SYNTH outbound: harness surfacing the two local-dev synthetic
    /// `outbound_delivery_*` capabilities over an injected
    /// [`FakeOutboundPreferencesFacade`] double.
    /// `create_capability_port` injects them via
    /// `apply_synthetic_capability_wrappers` because
    /// `outbound_target_tools` is `Some`. `target_set` runs with
    /// `requires_approval = true`, so its settings decision is exercised for
    /// real: global auto-approve (default ON) → `Allow`; a `Disabled` tool
    /// override (`disable_outbound_target_set_tool`) → `Deny`; auto-approve
    /// disabled → `Ask` (approval gate). The RETURNED harness leaves global
    /// auto-approve at its default-ON state so the happy/`NotFound` arms
    /// dispatch through `Allow`; the gate arm disables it per-test.
    pub(crate) async fn outbound_target_tools() -> HarnessResult<Self> {
        let facade =
            super::outbound_preferences::FakeOutboundPreferencesFacade::with_default_targets();
        Self::new_with_options(
            "reborn-e2e-outbound-target-tools",
            vec![
                CapabilityId::new(
                    ironclaw_reborn_composition::test_support::OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID,
                )?,
                CapabilityId::new(
                    ironclaw_reborn_composition::test_support::OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID,
                )?,
            ],
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ExternalWrite,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
            ],
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-outbound-target-user")?,
            HostRuntimeHarnessOptions::new(
                MountView::default(),
                Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                    true,
                )?),
            )
            .with_outbound_target_tools(facade, true),
        )
        .await
    }

    /// Group whose ONLY capability is `builtin.profile_set` (E-PROFILE seam).
    /// Uses `new_with_options` (not `core_builtin_tools_from_runtime`), so
    /// `profile_filesystem` is populated from `services.local_dev_profile_filesystem_for_test()`
    /// — the read-back half of the round trip a `RebornIntegrationGroup::profile_tools()`
    /// scenario needs. Base mounts are `/memory` directly (this harness's only
    /// capability needs it; no per-capability mount override required, unlike
    /// `core_builtin_tools_from_runtime`'s multi-capability surface).
    pub(crate) async fn profile_tools() -> HarnessResult<Self> {
        let harness = Self::new_with_options(
            "reborn-e2e-profile-tools",
            vec![CapabilityId::new(PROFILE_SET_CAPABILITY_ID)?],
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
            ],
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-profile-tools-user")?,
            HostRuntimeHarnessOptions::new(
                memory_mounts(MountPermissions::read_write_list_delete())?,
                Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                    true,
                )?),
            ),
        )
        .await?;
        harness
            .enable_global_auto_approve_for_product_and_harness_users()
            .await?;
        Ok(harness)
    }

    /// Group with NO first-party capability dispatch — the test drives the
    /// C-ATTACH seam purely through the attachment read port + inbound lander,
    /// never a tool call. Uses `new_with_options` (mirrors `profile_tools()`),
    /// so `attachment_test_support` is populated from
    /// `services.local_dev_attachment_test_support_for_test()`. No mounts needed:
    /// attachment landing/reading goes through `local_runtime.workspace_filesystem`
    /// directly, not the capability-dispatch `MountView` (mirrors
    /// `trigger_management_tools()`'s `MountView::default()`, which also has no
    /// filesystem capability to gate).
    pub(crate) async fn attachment_tools() -> HarnessResult<Self> {
        Self::new_with_options(
            "reborn-e2e-attachment-tools",
            Vec::new(),
            vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-attachment-tools-user")?,
            HostRuntimeHarnessOptions::new(
                MountView::default(),
                Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                    true,
                )?),
            ),
        )
        .await
    }

    /// `pub(crate)`: also used by `RebornIntegrationGroupBuilder::skill_management_tools`
    /// (`group_constructors.rs`, C-SKILL) to wire the SAME preset onto the
    /// int-tier group, so the QA/trace-tier smoke test and the int-tier group
    /// never drift on capability ids / mounts / policy.
    pub(crate) async fn skill_management_tools() -> HarnessResult<Self> {
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
        harness
            .enable_global_auto_approve_for_product_and_harness_users()
            .await?;
        Ok(harness)
    }

    /// Harness surfacing the local-dev synthetic `skill_activate` capability
    /// (E-SKILL seam). `new_with_options` builds the `skill_activation_source`
    /// (because `SKILL_ACTIVATE_CAPABILITY_ID` is in the allowlist) under
    /// `tenant` — the caller's ACTUAL group run-scope tenant, passed through
    /// rather than re-hardcoded here — which `create_capability_port` wraps
    /// onto the port and `into_group` wires as the runtime's
    /// `skill_context_source`. The skill file the model activates is seeded as
    /// a system-scoped skill by `RebornIntegrationGroup::skill_activation_tools`.
    /// Mirrors `skill_management_tools`/`project_tools`.
    pub(crate) async fn skill_activation_tools(tenant: &TenantId) -> HarnessResult<Self> {
        let mut harness = Self::new_with_options(
            "reborn-e2e-skill-activation-tools",
            vec![CapabilityId::new(
                ironclaw_reborn_composition::test_support::SKILL_ACTIVATE_CAPABILITY_ID,
            )?],
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::Network,
            ],
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-skill-activation-user")?,
            HostRuntimeHarnessOptions::new(
                skill_mounts()?,
                Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                    true,
                )?),
            )
            .with_skill_activation_tenant(tenant.clone()),
        )
        .await?;
        harness.network_policy = http_test_policy();
        harness
            .enable_global_auto_approve_for_product_and_harness_users()
            .await?;
        Ok(harness)
    }

    pub(crate) async fn trigger_management_tools() -> HarnessResult<Self> {
        let harness = Self::new_with_options(
            "reborn-e2e-trigger-management-tools",
            vec![
                CapabilityId::new(TRIGGER_CREATE_CAPABILITY_ID)?,
                CapabilityId::new(TRIGGER_LIST_CAPABILITY_ID)?,
                CapabilityId::new(TRIGGER_PAUSE_CAPABILITY_ID)?,
                CapabilityId::new(TRIGGER_RESUME_CAPABILITY_ID)?,
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
        .await?;
        harness
            .enable_global_auto_approve_for_product_and_harness_users()
            .await?;
        Ok(harness)
    }

    async fn enable_global_auto_approve_for_product_and_harness_users(&self) -> HarnessResult<()> {
        let product_scope = product_scope();
        self.enable_global_auto_approve(product_scope.clone())
            .await?;
        let mut harness_user_scope = product_scope;
        harness_user_scope.user_id = self.user_id.clone();
        self.enable_global_auto_approve(harness_user_scope).await?;
        Ok(())
    }

    pub(crate) async fn enable_global_auto_approve(
        &self,
        scope: ResourceScope,
    ) -> HarnessResult<()> {
        let store = self
            .auto_approve_settings
            .as_ref()
            .ok_or("host runtime harness missing local-dev auto-approve settings")?;
        store
            .set(AutoApproveSettingInput {
                updated_by: Principal::User(scope.user_id.clone()),
                scope,
                enabled: true,
            })
            .await?;
        Ok(())
    }

    /// Global auto-approve now defaults ON. A test that needs to exercise the
    /// per-tool approval gate must flip it OFF for the product and harness-user
    /// scopes the run authorizes against, as an explicit precondition.
    pub async fn disable_global_auto_approve_for_product_and_harness_users(
        &self,
    ) -> HarnessResult<()> {
        let product_scope = product_scope();
        self.disable_global_auto_approve(product_scope.clone())
            .await?;
        let mut harness_user_scope = product_scope;
        harness_user_scope.user_id = self.user_id.clone();
        self.disable_global_auto_approve(harness_user_scope).await?;
        Ok(())
    }

    pub(crate) async fn disable_global_auto_approve(
        &self,
        scope: ResourceScope,
    ) -> HarnessResult<()> {
        let store = self
            .auto_approve_settings
            .as_ref()
            .ok_or("host runtime harness missing local-dev auto-approve settings")?;
        store
            .set(AutoApproveSettingInput {
                updated_by: Principal::User(scope.user_id.clone()),
                scope,
                enabled: false,
            })
            .await?;
        Ok(())
    }

    async fn trace_commons_tools() -> HarnessResult<Self> {
        let mut harness = Self::new_with_options(
            "reborn-e2e-trace-commons-tools",
            vec![
                CapabilityId::new(TRACE_COMMONS_ONBOARD_CAPABILITY_ID)?,
                CapabilityId::new(TRACE_COMMONS_STATUS_CAPABILITY_ID)?,
                CapabilityId::new(TRACE_COMMONS_CREDITS_CAPABILITY_ID)?,
                CapabilityId::new(TRACE_COMMONS_PROFILE_TOKEN_CAPABILITY_ID)?,
                CapabilityId::new(TRACE_COMMONS_PROFILE_SET_CAPABILITY_ID)?,
            ],
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                // onboard persists device-key material (Ed25519 keypair +
                // policy.json) and profile_token writes profile_token.jwt, so
                // the harness allow-set must grant WriteFilesystem or those
                // capabilities are filtered out of the model-visible surface.
                EffectKind::WriteFilesystem,
                EffectKind::Network,
                EffectKind::ExternalWrite,
            ],
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-trace-commons-user")?,
            // The Trace Commons write/network capabilities are
            // PermissionMode::Ask (onboard, profile_token, profile_set) — like
            // the skill/trigger harnesses, the scripted run enables global
            // auto-approve so it is not gated.
            HostRuntimeHarnessOptions::new(
                MountView::default(),
                Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                    true,
                )?),
            ),
        )
        .await?;
        // onboard declares EffectKind::Network, so the lease must carry a
        // non-empty network policy or the obligation check rejects dispatch
        // before the consent gate runs.
        harness.network_policy = http_test_policy();
        harness
            .enable_global_auto_approve_for_product_and_harness_users()
            .await?;
        Ok(harness)
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
            seed_extension_credentials,
            skill_activation_tenant,
            outbound_target_facade,
            network_http_egress_for_test,
            activate_bundled_extensions_for_test,
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
        if let Some(egress) = network_http_egress_for_test {
            input = input.with_network_http_egress_for_test(egress);
        }
        let services = build_reborn_services(input).await?;
        if seed_extension_credentials {
            seed_extension_lifecycle_credentials(&services, &user_id).await?;
        }
        // C-JOURNEY: publish bundled WASM packages into the active-extension
        // registry directly (see `HostRuntimeHarnessOptions::activate_bundled_extensions_for_test`
        // doc) so their capabilities are genuinely dispatchable, not merely
        // granted at the harness-authority layer.
        for package in &activate_bundled_extensions_for_test {
            services
                .publish_bundled_extension_for_test(package)
                .ok_or(
                    "local-dev Reborn services missing extension management for test publish",
                )??;
        }
        let approval_parts = services.local_dev_approval_test_parts();
        let auto_approve_settings = services.local_dev_auto_approve_settings_for_test();
        // Capture the profile filesystem + project service + attachment support
        // before `services.host_runtime` is moved out below (E-PROFILE / E-PROJ /
        // C-ATTACH seams).
        let profile_filesystem = services.local_dev_profile_filesystem_for_test();
        let project_service = services.local_dev_project_service_for_test();
        // C-JOURNEY: capture product-auth before `services.host_runtime` is
        // moved out below, so `seed_github_credential_account` can create a
        // real credential account later (auth-gate happy-path resume).
        let product_auth = services.product_auth.clone();
        // E-SKILL: build the local-dev skill context source only when this
        // harness surfaces the synthetic `skill_activate` capability (i.e.
        // `skill_activation_tools`). Built with the caller-supplied tenant
        // (`HostRuntimeHarnessOptions::with_skill_activation_tenant`, sourced
        // from the group's actual run-scope tenant) so activation visibility
        // matches the turn's scope. Must precede the `services.host_runtime`
        // move (it borrows `&services`).
        let skill_activation_source = if capability_ids.iter().any(|id| {
            id.as_str() == ironclaw_reborn_composition::test_support::SKILL_ACTIVATE_CAPABILITY_ID
        }) {
            let tenant = skill_activation_tenant
                .ok_or("skill_activation_tools harness requires with_skill_activation_tenant")?;
            ironclaw_reborn_composition::test_support::build_local_dev_skill_context_source_for_test(
                &services, &tenant, true,
            )
        } else {
            None
        };
        let attachment_test_support = services.local_dev_attachment_test_support_for_test();
        // C-SYNTH outbound: pair the injected facade double with the local-dev
        // settings stores production's `outbound_delivery_capabilities` consumes,
        // captured from `RebornServices` before the `host_runtime` move. Only
        // `outbound_target_tools()` supplies the facade.
        let outbound_target_tools = match outbound_target_facade {
            Some((facade, requires_approval)) => {
                let tool_permission_overrides = services
                    .local_dev_tool_permission_overrides_for_test()
                    .ok_or("outbound_target_tools requires a local-dev tool-override store")?;
                let persistent_approval_policies = services
                    .local_dev_persistent_approval_policies_for_test()
                    .ok_or("outbound_target_tools requires a local-dev persistent-policy store")?;
                Some(OutboundTargetToolsParts {
                    facade,
                    requires_approval,
                    tool_permission_overrides,
                    persistent_approval_policies,
                })
            }
            None => None,
        };
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
            auto_approve_settings,
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
            additional_provider_trust: Vec::new(),
            user_id,
            invocations: Arc::new(Mutex::new(Vec::new())),
            results: Arc::new(Mutex::new(Vec::new())),
            http_egress: None,
            network_egress: None,
            process_port: None,
            profile_filesystem,
            project_service,
            skill_activation_source,
            attachment_test_support,
            outbound_target_tools,
            scope_capability_by_run_owner: false,
            product_auth,
        })
    }

    pub(crate) async fn core_builtin_tools() -> HarnessResult<Self> {
        Self::core_builtin_tools_with_network_policy(http_test_policy()).await
    }

    async fn core_builtin_tools_with_network_policy(
        network_policy: NetworkPolicy,
    ) -> HarnessResult<Self> {
        Self::core_builtin_tools_with_network_policy_and_process_port(network_policy, true).await
    }

    /// Variant used by `.with_live_shell()`: same as `core_builtin_tools_with_network_policy`
    /// but opts out of the recording process port so the real `LocalHostProcessPort`
    /// executes shell commands on the host.
    pub(crate) async fn core_builtin_tools_with_live_shell() -> HarnessResult<Self> {
        Self::core_builtin_tools_with_network_policy_and_process_port(http_test_policy(), false)
            .await
    }

    async fn core_builtin_tools_with_network_policy_and_process_port(
        network_policy: NetworkPolicy,
        recording_process: bool,
    ) -> HarnessResult<Self> {
        let (root, storage_root, workspace_root) = host_runtime_storage_roots()?;
        let runtime_http_egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
            br#"{"accepted":true}"#.to_vec(),
        ));
        // Slice 5: inject the inert recording port by default so `builtin.shell`
        // invocations in tests never spawn a real OS process. The `.with_live_shell()`
        // opt-in passes `recording_process = false`, which skips injection and lets
        // `HostRuntimeServices` default to the real `LocalHostProcessPort`.
        let recording_process_port = if recording_process {
            Some(Arc::new(super::process::RecordingProcessPort::new()))
        } else {
            None
        };
        let process_port_dyn: Option<Arc<dyn RuntimeProcessPort>> = recording_process_port
            .as_ref()
            .map(|p| Arc::clone(p) as Arc<dyn RuntimeProcessPort>);
        let runtime = local_dev_host_runtime_with_http_egress(
            storage_root.clone(),
            Arc::clone(&runtime_http_egress),
            process_port_dyn,
        )?;
        let mut harness = Self::core_builtin_tools_from_runtime(
            root,
            workspace_root,
            runtime,
            network_policy,
            UserId::new("reborn-e2e-core-builtins-user")?,
        )?;
        harness.http_egress = Some(runtime_http_egress);
        harness.process_port = recording_process_port;
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
            // profile_set writes to the memory mount (context/profile.json under
            // the user-scoped scope), so it needs the memory mount override just
            // like the four memory_* capabilities above.
            CapabilityId::new(PROFILE_SET_CAPABILITY_ID)?,
        ];
        Ok(Self {
            runtime,
            approval_parts: None,
            auto_approve_settings: None,
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
                CapabilityId::new(PROFILE_SET_CAPABILITY_ID)?,
                CapabilityId::new(READ_FILE_CAPABILITY_ID)?,
                CapabilityId::new(APPLY_PATCH_CAPABILITY_ID)?,
                // slice 5: `builtin.shell` on the surface so scripted shell calls
                // route through the process port (recording by default, live via
                // `.with_live_shell()`).
                CapabilityId::new(SHELL_CAPABILITY_ID)?,
            ],
            runtime_kind: RuntimeKind::FirstParty,
            effect_kinds: vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::Network,
                EffectKind::SpawnProcess,
                // slice 5: `builtin.shell` declares ExecuteCode; the grant's
                // allowed_effects must include it or the authorizer denies the
                // capability before it reaches the process port.
                EffectKind::ExecuteCode,
            ],
            network_policy,
            secrets: Vec::new(),
            provider_id: ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            additional_provider_trust: Vec::new(),
            user_id,
            invocations: Arc::new(Mutex::new(Vec::new())),
            results: Arc::new(Mutex::new(Vec::new())),
            http_egress: None,
            network_egress: None,
            process_port: None,
            profile_filesystem: None,
            project_service: None,
            skill_activation_source: None,
            attachment_test_support: None,
            outbound_target_tools: None,
            scope_capability_by_run_owner: false,
            product_auth: None,
        })
    }

    /// Wires the GitHub first-party WASM capabilities behind `GithubHarnessAuthorizer`.
    /// See `github_issue_tools_with_credential_result` for the credential-injection
    /// coupling this relies on (T0-SECRET-INJECT).
    pub(crate) async fn github_issue_tools() -> HarnessResult<Self> {
        // Credential account resolves to a real handle → capability dispatches.
        Self::github_issue_tools_with_credential_result(Ok(SecretHandle::new(
            "github_manual_access",
        )?))
    }

    /// E-AUTHGATE: the GitHub extension wired so its credential account resolver
    /// returns `AuthRequired`, raising a `TurnStatus::BlockedAuth` gate when a
    /// `github.*` capability is dispatched. Used by `RebornIntegrationGroup::live_auth_gate`.
    pub(crate) async fn github_issue_tools_auth_required() -> HarnessResult<Self> {
        Self::github_issue_tools_with_credential_result(Err(CredentialStageError::AuthRequired))
    }

    /// Shared GitHub-extension constructor (E-AUTHGATE): the only difference
    /// between the happy-path and auth-blocked variants is the credential account
    /// resolver result, so the full `Self {..}` literal lives here once.
    ///
    /// **Credential injection runs through two mechanisms here, not one — worth
    /// knowing before you change either.** The authorizer's
    /// `InjectCredentialAccountOnce` obligation is one path. The
    /// `local_dev_host_runtime_with_registry_and_egress` helper this calls into
    /// separately auto-wires `SharedHostWasmRuntimeCredentials` with product-auth
    /// restaging via `try_with_wasm_runtime` (since both `.with_secret_store` and
    /// `.with_runtime_credential_account_resolver` are always set on that path),
    /// which independently resolves the GitHub manifest's declared
    /// `runtime_credentials` and stages the same secret. That staging path runs
    /// unconditionally on every WASM HTTP call (`WasmRuntimeHttpAdapter::request`)
    /// — it is not gated on the authorizer's `Decision`. So a test asserting on the
    /// injected header proves the *end-to-end* wire outcome, not that the
    /// authorizer's obligation specifically is the sole producer of the header.
    ///
    /// As currently wired (manually verified once, not re-checked by CI — treat as
    /// current-harness observation, not a guaranteed contract): removing the
    /// obligation does not make the call fall back to an unauthenticated request;
    /// the run instead hangs and never reaches `Completed`. That's why the
    /// mutation-verify in `reborn_integration_secret_injection.rs` proves the
    /// obligation's secret reaches the wire by flipping the secret *value* (a fast,
    /// specific assertion failure) rather than by removing the obligation (which
    /// would only yield a slow, ambiguous timeout — a poor mutation-test signal).
    fn github_issue_tools_with_credential_result(
        credential_account_result: Result<SecretHandle, CredentialStageError>,
    ) -> HarnessResult<Self> {
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
            credential_account_result,
        )?;
        let mounts = workspace_mounts(MountPermissions::read_write_list_delete())?;
        Ok(Self {
            runtime,
            approval_parts: None,
            auto_approve_settings: None,
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
            additional_provider_trust: Vec::new(),
            user_id: UserId::new("reborn-e2e-github-user")?,
            invocations: Arc::new(Mutex::new(Vec::new())),
            results: Arc::new(Mutex::new(Vec::new())),
            http_egress: Some(runtime_http_egress),
            network_egress: Some(network_egress),
            process_port: None,
            profile_filesystem: None,
            project_service: None,
            skill_activation_source: None,
            attachment_test_support: None,
            outbound_target_tools: None,
            scope_capability_by_run_owner: false,
            product_auth: None,
        })
    }

    /// Slice 6: wire a single MCP capability backed by the loopback mock server.
    ///
    /// `mcp_url`  — the mock server's MCP endpoint (e.g. `"http://127.0.0.1:PORT/mcp"`).
    /// `provider_id`   — extension id used in the registry (e.g. `"mock-mcp"`).
    /// `capability_id` — capability id surfaced to the model (e.g. `"mock-mcp.search"`).
    ///
    /// The harness (via the `harness_mcp` scaffolding) builds a loopback MCP
    /// egress that makes REAL HTTP connections to the mock server, injecting a
    /// fake Bearer token to satisfy the mock's auth gate. Production egress
    /// policy, network policy, and credential stores are bypassed — this path is
    /// test-only.
    pub(crate) async fn mock_mcp_tools(
        mcp_url: &str,
        provider_id: &str,
        capability_id: &str,
    ) -> HarnessResult<Self> {
        let (root, storage_root, workspace_root) = host_runtime_storage_roots()?;
        // Recording egress for any first-party tool paths (unused in MCP tests,
        // but HostRuntimeServices requires it when first_party_capabilities are wired).
        let first_party_egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
            br#"{"accepted":true}"#.to_vec(),
        ));
        // Real loopback egress + MCP runtime for the mock MCP server; the
        // scaffolding (egress, adapter chain, runtime) lives in `harness_mcp`.
        let mcp_runtime = build_loopback_mcp_runtime(mcp_url)?;
        let mut registry = ExtensionRegistry::new();
        registry.insert(mock_mcp_extension_package(
            provider_id,
            mcp_url,
            capability_id,
        )?)?;
        let runtime = local_dev_host_runtime_with_registry_egress_and_mcp(
            storage_root,
            registry,
            Arc::clone(&first_party_egress),
            mcp_runtime,
            provider_id,
        )?;
        let mounts = workspace_mounts(MountPermissions::read_write_list_delete())?;
        Ok(Self {
            runtime,
            approval_parts: None,
            auto_approve_settings: None,
            pending_approval_scopes: Arc::new(Mutex::new(HashMap::new())),
            io: Arc::new(ProductLiveCapabilityIo::default()),
            root,
            workspace_root,
            mounts,
            capability_mount_overrides: Vec::new(),
            capability_ids: vec![CapabilityId::new(capability_id)?],
            runtime_kind: RuntimeKind::Mcp,
            effect_kinds: vec![EffectKind::DispatchCapability, EffectKind::Network],
            // The MCP capability declares `EffectKind::Network`, so authorization
            // attaches an `ApplyNetworkPolicy` obligation that the host runtime
            // rejects when `allowed_targets` is empty (a default `NetworkPolicy`).
            // The mock server lives at `http://127.0.0.1:<port>/mcp`, so permit the
            // loopback host (and disable the private-IP denial that would otherwise
            // block 127.0.0.1) so the MCP egress reaches the loopback server.
            network_policy: mcp_loopback_network_policy(),
            secrets: Vec::new(),
            provider_id: ExtensionId::new(provider_id)?,
            additional_provider_trust: Vec::new(),
            user_id: UserId::new("reborn-itest-mcp-user")?,
            invocations: Arc::new(Mutex::new(Vec::new())),
            results: Arc::new(Mutex::new(Vec::new())),
            http_egress: None,
            network_egress: None,
            process_port: None,
            profile_filesystem: None,
            project_service: None,
            skill_activation_source: None,
            attachment_test_support: None,
            outbound_target_tools: None,
            scope_capability_by_run_owner: false,
            product_auth: None,
        })
    }

    /// C-WEBACCESS: wires the real first-party `web-access.search` /
    /// `web-access.get_content` capabilities via the production
    /// `register_bundled_web_access_first_party_handlers` registration
    /// (`harness_web_access.rs`), which dispatches through the same
    /// `WebAccessExecutor` production composition uses. Unlike
    /// `github_issue_tools`, no credential-injecting authorizer is needed —
    /// web-access declares zero `runtime_credentials` — so this wires the
    /// plain default `GrantAuthorizer`.
    ///
    /// The three-leg Exa MCP handshake (`initialize` → `notifications/initialized`
    /// → `tools/call`) all target the same URL, so script it via
    /// `RecordingRuntimeHttpEgress::push_response_body` (FIFO), not the keyed
    /// matcher — see [`install_web_access_responses`](Self::install_web_access_responses),
    /// called from `RebornIntegrationHarnessBuilder::build` before the harness
    /// is returned.
    pub(crate) async fn web_access_tools() -> HarnessResult<Self> {
        let (root, storage_root, workspace_root) = host_runtime_storage_roots()?;
        let http_egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
            br#"{"accepted":true}"#.to_vec(),
        ));
        let mut registry = ExtensionRegistry::new();
        registry.insert(harness_web_access::web_access_extension_package()?)?;
        let runtime = harness_web_access::local_dev_host_runtime_with_web_access(
            storage_root,
            registry,
            Arc::clone(&http_egress),
        )?;
        let mounts = workspace_mounts(MountPermissions::read_write_list_delete())?;
        Ok(Self {
            runtime,
            approval_parts: None,
            auto_approve_settings: None,
            pending_approval_scopes: Arc::new(Mutex::new(HashMap::new())),
            io: Arc::new(ProductLiveCapabilityIo::default()),
            root,
            workspace_root,
            mounts,
            capability_mount_overrides: Vec::new(),
            capability_ids: vec![
                CapabilityId::new(WEB_SEARCH_CAPABILITY_ID)?,
                CapabilityId::new(WEB_GET_CONTENT_CAPABILITY_ID)?,
            ],
            runtime_kind: RuntimeKind::FirstParty,
            effect_kinds: vec![EffectKind::DispatchCapability, EffectKind::Network],
            network_policy: harness_web_access::exa_mcp_test_network_policy(),
            secrets: Vec::new(),
            provider_id: ExtensionId::new(harness_web_access::WEB_ACCESS_PROVIDER_ID)?,
            additional_provider_trust: Vec::new(),
            user_id: UserId::new("reborn-itest-web-access-user")?,
            invocations: Arc::new(Mutex::new(Vec::new())),
            results: Arc::new(Mutex::new(Vec::new())),
            http_egress: Some(http_egress),
            network_egress: None,
            process_port: None,
            profile_filesystem: None,
            project_service: None,
            skill_activation_source: None,
            attachment_test_support: None,
            outbound_target_tools: None,
            scope_capability_by_run_owner: false,
            product_auth: None,
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

    /// Install FIFO response bodies (C-WEBACCESS) onto the recording runtime
    /// HTTP egress, consumed in call order ahead of the default body. Mirrors
    /// [`install_http_responses`](Self::install_http_responses)'s shape but for
    /// the web-access backend's `push_response_body` FIFO queue rather than the
    /// keyed matcher — the three-leg Exa MCP handshake (`initialize` →
    /// `notifications/initialized` → `tools/call`) all target the same
    /// URL/method/capability, so only the FIFO queue can script them
    /// independently. Errors if this harness wired no recording egress. Called
    /// from `RebornIntegrationHarnessBuilder::build` (build-time only — no
    /// post-build mutation).
    pub(crate) fn install_web_access_responses(
        &self,
        bodies: impl IntoIterator<Item = Vec<u8>>,
    ) -> HarnessResult<()> {
        let egress = self
            .http_egress
            .as_ref()
            .ok_or("web-access host runtime has no recording egress wired")?;
        for body in bodies {
            egress.push_response_body(body);
        }
        Ok(())
    }

    /// Snapshot of every command string recorded by the inert process port
    /// (slice 5). Empty when the harness uses the live `LocalHostProcessPort`
    /// (`.with_live_shell()` path) or predates slice 5.
    fn process_commands(&self) -> Vec<String> {
        self.process_port
            .as_ref()
            .map(|port| port.commands())
            .unwrap_or_default()
    }

    /// Install URL/method/capability-keyed scripted responses into the recording
    /// HTTP egress (§3.6 P1 ergonomics). Errors if this harness wired no
    /// recording egress (e.g. the live-HTTP variant).
    pub(crate) fn install_http_responses(
        &self,
        responses: impl IntoIterator<Item = super::http_matcher::ScriptedHttpResponse>,
    ) -> HarnessResult<()> {
        self.http_egress
            .as_ref()
            .ok_or("host runtime harness has no recording http egress to script")?
            .install_scripted(responses);
        Ok(())
    }

    /// Install a sticky scripted `builtin.shell` process result on the inert
    /// recording process port (mirrors `install_http_responses`). Errors if the
    /// harness has no recording port (e.g. the `.with_live_shell()` path).
    pub(crate) fn install_process_script(
        &self,
        result: super::process::ScriptedProcessResult,
    ) -> HarnessResult<()> {
        self.process_port
            .as_ref()
            .ok_or("host runtime harness has no recording process port to script")?
            .set_scripted(result);
        Ok(())
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
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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

    /// Deny a pending local-dev approval gate (the model-declined path). Mirrors
    /// [`approve_local_dev_gate`](Self::approve_local_dev_gate) but resolves the
    /// persisted request to `Denied` (no lease issued) via `ApprovalResolver::deny`.
    /// The caller then resumes the run with `GateResumeDisposition::Denied` so the
    /// executor surfaces a non-retryable authorization failure to the model.
    async fn deny_local_dev_gate(&self, gate_ref: &GateRef) -> HarnessResult<()> {
        let approval_parts = self
            .approval_parts
            .as_ref()
            .ok_or("host runtime harness has no local-dev approval stores")?;
        let request_id = approval_request_id_from_gate_ref(gate_ref)?;
        let scope = self
            .pending_approval_scopes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&request_id)
            .cloned()
            .ok_or("approval gate was not recorded by the host runtime harness")?;
        let resolver = ApprovalResolver::new(
            approval_parts.approval_requests.as_ref(),
            approval_parts.capability_leases.as_ref(),
        );
        resolver
            .deny(
                &scope,
                request_id,
                DenyApproval {
                    denied_by: Principal::User(scope.user_id.clone()),
                },
            )
            .await?;
        Ok(())
    }

    /// The persisted approval-request store, when this harness wires the real
    /// local-dev approval stores (`file_tools_requiring_approval`). The
    /// integration runtime builds an [`ApprovalGateEvidenceStore`] over it so a
    /// `BlockedApproval` run is verified at loop exit (mirrors production
    /// `runtime.rs:2799`) and genuinely pauses instead of failing.
    pub(crate) fn approval_requests_store(
        &self,
    ) -> Option<Arc<dyn ironclaw_run_state::ApprovalRequestStore>> {
        self.approval_parts
            .as_ref()
            .map(|parts| Arc::clone(&parts.approval_requests))
    }

    /// The user id this capability harness's first-party tools execute under.
    /// The dispatch-time auto-approve check is keyed `(tenant, user)` on THIS
    /// user (not the run's binding owner), so the group derives the auto-approve
    /// scope from it — see `GroupSharedStorage::auto_approve_scope`.
    pub(crate) fn user_id(&self) -> &UserId {
        &self.user_id
    }

    /// E-PROFILE: the raw local-dev memory filesystem backing the user-profile
    /// source, for write→read-back assertions on `context/profile.json`. `Some`
    /// only for `new_with_options`-built harnesses. Consumed by the E-PROFILE
    /// `profile_tools()` constructor and the `reborn_integration_profile` test.
    pub(crate) fn profile_filesystem_for_test(&self) -> Option<Arc<dyn RootFilesystem>> {
        self.profile_filesystem.clone()
    }

    /// E-PROJ: the project service backing the synthetic `project_create`
    /// capability, for write→read-back assertions — mirrors
    /// `profile_filesystem_for_test`'s role for E-PROFILE. `Some` only for
    /// `project_tools()`-built harnesses. Lets a test read a created project
    /// back through the SAME `Arc<dyn ProjectService>` instance
    /// `apply_synthetic_capability_wrappers` dispatches writes through, rather
    /// than reconstructing an equivalent (and possibly unwritten) one.
    pub(crate) fn project_service_for_test(&self) -> Option<Arc<dyn ProjectService>> {
        self.project_service.clone()
    }

    /// C-SYNTH outbound: the injected facade double, for read-back that a
    /// `target_set` actually reached the facade seam
    /// (`recorded_set_target_ids`). `Some` only for `outbound_target_tools()`.
    pub(crate) fn outbound_preferences_facade_for_test(
        &self,
    ) -> Option<Arc<super::outbound_preferences::FakeOutboundPreferencesFacade>> {
        self.outbound_target_tools
            .as_ref()
            .map(|parts| Arc::clone(&parts.facade))
    }

    /// C-SYNTH outbound: persist a `Disabled` per-tool permission override for
    /// `outbound_delivery_target_set` under `(tenant, user)`, driving the
    /// handler's settings decision to `Deny` → `Failed{policy_denied}`. The
    /// scope must be the run's EFFECTIVE dispatch user (the thread binding actor,
    /// `harness.binding.actor_user_id`) — the same `(tenant, user)`
    /// `StoreApprovalSettingsProvider::tool_override` reads it back under
    /// (`PersistentApprovalScope` = tenant+user, invocation-independent). `Some`
    /// only for `outbound_target_tools()`.
    pub(crate) async fn disable_outbound_target_set_tool(
        &self,
        tenant_id: TenantId,
        user_id: UserId,
    ) -> HarnessResult<()> {
        let parts = self
            .outbound_target_tools
            .as_ref()
            .ok_or("harness has no outbound_target_tools backing store")?;
        let scope = ResourceScope {
            tenant_id,
            user_id: user_id.clone(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        parts
            .tool_permission_overrides
            .set(ironclaw_approvals::CapabilityPermissionOverrideInput {
                scope,
                capability_id: CapabilityId::new(
                    ironclaw_reborn_composition::test_support::OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID,
                )?,
                state: ironclaw_approvals::CapabilityPermissionOverride::Disabled,
                updated_by: Principal::User(user_id),
            })
            .await?;
        Ok(())
    }

    /// E-SKILL: the `HostSkillContextSource` to wire as the runtime's
    /// `skill_context_source` in `into_group`, so activated-skill instructions
    /// inject into the model request. `Some` only for `skill_activation_tools()`.
    pub(crate) fn skill_context_source_for_test(
        &self,
    ) -> Option<Arc<dyn ironclaw_loop_support::HostSkillContextSource>> {
        self.skill_activation_source
            .as_ref()
            .map(|source| source.context_source())
    }

    /// E-DURABLE: the on-disk local-dev storage root this harness's capability
    /// stores persist under (`<tempdir>/local-dev`). Mirrors the `storage_root`
    /// computed inline in `new_with_options`. A durability test reopens a fresh,
    /// independent store at this path (see
    /// `open_local_dev_extension_installation_store_for_test`) to prove capability
    /// state survives a reopen, paralleling `assert_reply_persists_after_reopen`.
    /// Tests only.
    pub(crate) fn storage_root_for_test(&self) -> PathBuf {
        self.root.path().join("local-dev")
    }

    /// C-DURABLE: resolve `gate_ref` (a `"gate:approval-<id>"` local-dev
    /// approval gate) to the `(ApprovalRequestId, ResourceScope)` pair a fresh,
    /// independently-reopened `ApprovalRequestStore::get`/`read_versioned` call
    /// needs. Reuses the SAME private lookup `approve_local_dev_gate`/
    /// `deny_local_dev_gate` already use (`approval_request_id_from_gate_ref` +
    /// `pending_approval_scopes`) so a durability test's scope construction can
    /// never drift from the live approve/deny path. Tests only.
    pub(crate) fn approval_request_scope_for_test(
        &self,
        gate_ref: &GateRef,
    ) -> HarnessResult<(ApprovalRequestId, ResourceScope)> {
        let request_id = approval_request_id_from_gate_ref(gate_ref)?;
        let scope = self
            .pending_approval_scopes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&request_id)
            .cloned()
            .ok_or("approval gate was not recorded by the host runtime harness")?;
        Ok((request_id, scope))
    }

    /// E-SKILL: seed a system-scoped skill on this harness's on-disk skill
    /// filesystem so the model can activate it (`skill_activate`/`$name`). Writes
    /// `<storage_root>/system/skills/<name>/SKILL.md` — the system bundle root is
    /// always present in the skills extension's roots regardless of the run's
    /// tenant/user (`FirstPartySkillsExtensionHandles::bundle_roots`), so both
    /// `activate_skills_for_run` (the `skill_activate` capability) and the
    /// runtime's `skill_context_source` resolve it deterministically without
    /// depending on the harness's run-scope owner resolution. User-scoped skill
    /// filesystem resolution is already covered by the runtime.rs suite; this
    /// seam only needs the skill to exist so the capability + context wiring can
    /// be driven. Mirrors the runtime-test system-skill layout
    /// (runtime.rs `system/skills/<name>/SKILL.md`). Tests only.
    pub(crate) fn seed_system_skill_for_test(
        &self,
        name: &str,
        description: &str,
        prompt: &str,
    ) -> HarnessResult<()> {
        let dir = self
            .storage_root_for_test()
            .join("system")
            .join("skills")
            .join(name);
        std::fs::create_dir_all(&dir)?;
        let body = format!(
            "---\nname: {name}\ndescription: {description}\nactivation:\n  keywords: [\"{name}\"]\n---\n\n{prompt}"
        );
        std::fs::write(dir.join("SKILL.md"), body)?;
        Ok(())
    }

    /// C-ATTACH: the attachment read port + inbound lander over this harness's
    /// local-dev workspace filesystem, for wiring `DefaultPlannedRuntimeParts.attachment_read_port`
    /// and `DefaultInboundTurnService::with_inbound_attachments` — mirrors
    /// `profile_filesystem_for_test`'s role for E-PROFILE.
    pub(crate) fn attachment_test_support_for_test(
        &self,
    ) -> Option<ironclaw_reborn_composition::AttachmentTestSupport> {
        self.attachment_test_support.clone()
    }

    /// E-PROJ: wrap `port` with the local-dev synthetic capabilities this harness
    /// surfaces, in one linear step (keeps the capability-specific knowledge out
    /// of `create_capability_port`'s main assembly chain).
    ///
    /// Partial synthetic wrap: `project_create` (E-PROJ), `skill_activate`
    /// (E-SKILL), and the two `outbound_delivery_*` capabilities (C-SYNTH
    /// outbound), each layered independently when this harness holds the backing
    /// handle, so other local-dev groups are unaffected. See
    /// `LocalDevCapabilityPortFactory::build_inner()` for the full production set.
    fn apply_synthetic_capability_wrappers(
        &self,
        port: Arc<dyn LoopCapabilityPort>,
        run_context: &LoopRunContext,
        input_resolver: Arc<dyn ironclaw_loop_support::LoopCapabilityInputResolver>,
        result_writer: Arc<dyn LoopCapabilityResultWriter>,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        let mut port = port;
        // project_create (E-PROJ): wrapped only for `project_tools`.
        if let Some(project_service) = &self.project_service
            && self.capability_ids.iter().any(|id| {
                id.as_str()
                    == ironclaw_reborn_composition::test_support::PROJECT_CREATE_CAPABILITY_ID
            })
        {
            port =
                ironclaw_reborn_composition::test_support::wrap_project_create_capability_for_test(
                    port,
                    Arc::clone(project_service),
                    self.user_id.clone(),
                    run_context.clone(),
                    input_resolver.clone(),
                    result_writer.clone(),
                )?;
        }
        // outbound_delivery_* (C-SYNTH outbound): wrapped only for
        // `outbound_target_tools`. The facade double is injected at the
        // production-wired trait seam; the settings/approval stores are the same
        // ones `outbound_delivery_capabilities` consumes in production (the
        // auto-approve + approval-request/lease stores are reused from the
        // sibling `auto_approve_settings` / `approval_parts` harness fields).
        if let Some(parts) = &self.outbound_target_tools {
            let approval = self.approval_parts.as_ref().ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "outbound_target_tools requires local-dev approval stores",
                )
            })?;
            let auto_approve = self.auto_approve_settings.clone().ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "outbound_target_tools requires the local-dev auto-approve store",
                )
            })?;
            // Record the synthetic capability's approval scope into
            // `pending_approval_scopes` (the host-runtime recorder can't see a
            // port-level synthetic gate) so `approve_local_dev_gate` /
            // `deny_local_dev_gate` resolve it; delegates to the inner store the
            // evidence/approve/deny paths read.
            let recording_approval_requests: Arc<dyn ironclaw_run_state::ApprovalRequestStore> =
                Arc::new(RecordingApprovalRequestStore {
                    inner: Arc::clone(&approval.approval_requests),
                    pending_approval_scopes: Arc::clone(&self.pending_approval_scopes),
                });
            port =
                ironclaw_reborn_composition::test_support::wrap_outbound_delivery_capabilities_for_test(
                    port,
                    ironclaw_reborn_composition::test_support::OutboundDeliveryCapabilityTestParts {
                        facade: Arc::clone(&parts.facade)
                            as Arc<dyn ironclaw_product_workflow::OutboundPreferencesProductFacade>,
                        fallback_user_id: self.user_id.clone(),
                        approval_requests: recording_approval_requests,
                        capability_leases: Arc::clone(&approval.capability_leases),
                        tool_permission_overrides: Arc::clone(&parts.tool_permission_overrides),
                        auto_approve,
                        persistent_policies: Arc::clone(&parts.persistent_approval_policies),
                        target_set_requires_approval: parts.requires_approval,
                        run_context: run_context.clone(),
                        input_resolver: input_resolver.clone(),
                        result_writer: result_writer.clone(),
                    },
                )?;
        }
        // skill_activate (E-SKILL): wrapped only for `skill_activation_tools`.
        if let Some(skill_source) = &self.skill_activation_source
            && self.capability_ids.iter().any(|id| {
                id.as_str()
                    == ironclaw_reborn_composition::test_support::SKILL_ACTIVATE_CAPABILITY_ID
            })
        {
            port =
                ironclaw_reborn_composition::test_support::wrap_skill_activation_capability_for_test(
                    port,
                    skill_source,
                    run_context.clone(),
                    input_resolver,
                    result_writer,
                )?;
        }
        Ok(port)
    }

    /// Override the user this capability harness executes first-party tools under.
    /// The dispatch ResourceScope, approval-request persistence, auto-approve
    /// keying, and the approval-gate-evidence lookup are ALL keyed on this user
    /// (`HostRuntimeHarnessCapabilityPortFactory` builds the authority from
    /// `self.user_id`). The integration harness sets it to the run's binding owner
    /// so capability dispatch and the turn run under the SAME `(tenant, user)` —
    /// matching production (where the run owner *is* the capability user) instead
    /// of the constructor's fixed test user. Without this, a `BlockedApproval`
    /// run's request persists under the capability user but the gate-evidence
    /// lookup uses the turn owner, so the gate is never verified and the run goes
    /// terminal `Failed`.
    pub(crate) fn with_user_id(mut self, user_id: UserId) -> Self {
        self.user_id = user_id;
        self
    }

    /// C-MULTIUSER: opt in to per-actor capability scoping. With this set,
    /// [`create_capability_port`] resolves the execution `(tenant, user)` from
    /// each run's OWN owner/actor rather than this harness's single fixed
    /// `user_id` — so N actors sharing one capability backend dispatch under N
    /// distinct scopes. See [`scope_capability_by_run_owner`] and
    /// [`dispatch_user_for_run`]. Enabled only by the multiuser group
    /// constructors; every other harness keeps the legacy fixed-user behavior.
    pub(crate) fn with_run_owner_scoped_capability_dispatch(mut self) -> Self {
        self.scope_capability_by_run_owner = true;
        self
    }

    /// The capability-execution `UserId` for one run. Mirrors production
    /// `local_dev_visible_capability_request`'s owner→actor→fallback resolution
    /// (`runtime/local_dev.rs`): when [`scope_capability_by_run_owner`] is set,
    /// prefer the run scope's explicit owner, then the run actor, then fall back
    /// to the fixed harness `user_id`. Without the flag, always the fixed
    /// `user_id` (legacy behavior — every existing test unaffected).
    fn dispatch_user_for_run(&self, run_context: &LoopRunContext) -> UserId {
        if self.scope_capability_by_run_owner {
            run_context
                .scope
                .explicit_owner_user_id()
                .cloned()
                .or_else(|| run_context.actor().map(|actor| actor.user_id.clone()))
                .unwrap_or_else(|| self.user_id.clone())
        } else {
            self.user_id.clone()
        }
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
            constraints: GrantConstraints {
                allowed_effects: self.effect_kinds.clone(),
                mounts,
                network: self.network_policy.clone(),
                secrets: self.secrets.clone(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
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

async fn seed_extension_lifecycle_credentials(
    services: &ironclaw_reborn_composition::RebornServices,
    user_id: &UserId,
) -> HarnessResult<()> {
    let product_auth = services
        .product_auth
        .as_ref()
        .ok_or("extension lifecycle harness missing product auth")?;
    let scope = AuthProductScope::credential_owner(
        &ResourceScope {
            tenant_id: TenantId::new("tenant-e2e")?,
            user_id: user_id.clone(),
            agent_id: Some(AgentId::new("agent-e2e")?),
            project_id: Some(ProjectId::new("project-e2e")?),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        },
        AuthSurface::Api,
    );
    let accounts = product_auth.credential_account_service();
    for seed in extension_lifecycle_credential_seeds() {
        accounts
            .create_account(NewCredentialAccount {
                scope: scope.clone(),
                provider: AuthProviderId::new(seed.provider)?,
                label: CredentialAccountLabel::new(seed.label)?,
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new(seed.secret_handle)?),
                refresh_secret: None,
                scopes: seed
                    .scopes
                    .iter()
                    .map(|scope| ProviderScope::new(*scope))
                    .collect::<Result<Vec<_>, _>>()?,
            })
            .await?;
    }
    Ok(())
}

struct ExtensionLifecycleCredentialSeed {
    provider: &'static str,
    label: &'static str,
    secret_handle: &'static str,
    scopes: &'static [&'static str],
}

fn extension_lifecycle_credential_seeds() -> &'static [ExtensionLifecycleCredentialSeed] {
    &[
        ExtensionLifecycleCredentialSeed {
            provider: "github",
            label: "qa github",
            secret_handle: "qa_github_access",
            scopes: &[],
        },
        ExtensionLifecycleCredentialSeed {
            provider: "google",
            label: "qa google",
            secret_handle: "qa_google_access",
            scopes: &[
                "https://www.googleapis.com/auth/calendar.events",
                "https://www.googleapis.com/auth/calendar.readonly",
                "https://www.googleapis.com/auth/documents",
                "https://www.googleapis.com/auth/documents.readonly",
                "https://www.googleapis.com/auth/drive",
                "https://www.googleapis.com/auth/drive.readonly",
                "https://www.googleapis.com/auth/gmail.modify",
                "https://www.googleapis.com/auth/gmail.readonly",
                "https://www.googleapis.com/auth/gmail.send",
                "https://www.googleapis.com/auth/presentations",
                "https://www.googleapis.com/auth/presentations.readonly",
                "https://www.googleapis.com/auth/spreadsheets",
                "https://www.googleapis.com/auth/spreadsheets.readonly",
            ],
        },
        ExtensionLifecycleCredentialSeed {
            provider: "nearai",
            label: "qa nearai",
            secret_handle: "qa_nearai_access",
            scopes: &[],
        },
        ExtensionLifecycleCredentialSeed {
            provider: "notion",
            label: "qa notion",
            secret_handle: "qa_notion_access",
            scopes: &[],
        },
    ]
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
                .unwrap_or_else(std::sync::PoisonError::into_inner)
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
                .unwrap_or_else(std::sync::PoisonError::into_inner)
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

    /// C-JOURNEY: forward auth-resume to the real runtime. `auth_resume_capability`
    /// is a DEFAULTED trait method whose default fails loudly ("capability
    /// auth-resume is unsupported by this host runtime", `ironclaw_host_runtime`
    /// lib.rs) precisely so wrappers that forget to forward it fail visibly —
    /// which is exactly what happened here: this wrapper predates any test
    /// exercising a happy-path auth resume, so the missing forward was latent
    /// until the first auth→resolve→re-dispatch journey drove it. Mirrors
    /// `invoke_capability`'s ApprovalRequired scope recording because
    /// `auth_resume_json` can itself raise an approval gate (the NoPriorLease
    /// path re-runs authorization).
    async fn auth_resume_capability(
        &self,
        request: ironclaw_host_runtime::RuntimeCapabilityAuthResumeRequest,
    ) -> Result<RuntimeCapabilityOutcome, HostRuntimeError> {
        let scope = request.context.resource_scope.clone();
        let outcome = self.inner.auth_resume_capability(request).await?;
        if let RuntimeCapabilityOutcome::ApprovalRequired(gate) = &outcome {
            self.pending_approval_scopes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(gate.approval_request_id, scope);
        }
        Ok(outcome)
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

/// Records `(ApprovalRequestId, ResourceScope)` on `save_pending`, then delegates
/// every method to the inner store. Synthetic local-dev capabilities (e.g.
/// `outbound_delivery_target_set`) persist their approval requests directly to
/// the approval store rather than through the host runtime, so
/// [`RecordingHostRuntime`] (which only observes host-runtime-level gates) never
/// captures their scope. Wrapping the store the synthetic capability writes
/// through restores the same `pending_approval_scopes` bookkeeping the
/// `approve_local_dev_gate` / `deny_local_dev_gate` lookups depend on. Delegation
/// is total, so the inner store the evidence/approve/deny paths read stays the
/// single source of truth.
struct RecordingApprovalRequestStore {
    inner: Arc<dyn ironclaw_run_state::ApprovalRequestStore>,
    pending_approval_scopes: Arc<Mutex<HashMap<ApprovalRequestId, ResourceScope>>>,
}

#[async_trait]
impl ironclaw_run_state::ApprovalRequestStore for RecordingApprovalRequestStore {
    async fn save_pending(
        &self,
        scope: ResourceScope,
        request: ironclaw_host_api::approval::ApprovalRequest,
    ) -> Result<ironclaw_run_state::ApprovalRecord, ironclaw_run_state::RunStateError> {
        self.pending_approval_scopes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(request.id, scope.clone());
        self.inner.save_pending(scope, request).await
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<Option<ironclaw_run_state::ApprovalRecord>, ironclaw_run_state::RunStateError> {
        self.inner.get(scope, request_id).await
    }

    async fn approve(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ironclaw_run_state::ApprovalRecord, ironclaw_run_state::RunStateError> {
        self.inner.approve(scope, request_id).await
    }

    async fn deny(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ironclaw_run_state::ApprovalRecord, ironclaw_run_state::RunStateError> {
        self.inner.deny(scope, request_id).await
    }

    async fn discard_pending(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
    ) -> Result<ironclaw_run_state::ApprovalRecord, ironclaw_run_state::RunStateError> {
        self.inner.discard_pending(scope, request_id).await
    }

    async fn records_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<ironclaw_run_state::ApprovalRecord>, ironclaw_run_state::RunStateError> {
        self.inner.records_for_scope(scope).await
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
        // C-MULTIUSER: resolve the execution user per run (owner/actor) when the
        // harness opts in, else the fixed harness user. Both the authority scope
        // and the grant grantee MUST use the SAME user so the lease is
        // self-consistent (grantee == execution user) — matching production.
        let dispatch_user = self.harness.dispatch_user_for_run(run_context);
        let mut authority = ProductLiveVisibleCapabilityRequestConfig::new(
            dispatch_user.clone(),
            self.harness.runtime_kind,
            TrustClass::FirstParty,
            SurfaceKind::new("agent_loop").map_err(host_runtime_harness_error)?,
            CapabilitySurfacePolicy::allow_all(),
        )
        .with_mounts(self.harness.mounts.clone())
        .with_grants(capability_grants(
            Principal::User(dispatch_user.clone()),
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
        for (provider, effects) in &self.harness.additional_provider_trust {
            authority = authority.with_provider_trust_for_effects(
                provider.clone(),
                EffectiveTrustClass::user_trusted(),
                effects.clone(),
            );
        }
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
            result_writer.clone(),
            milestone_sink,
        )
        .with_execution_mounts(execution_mounts);
        for (capability_id, mounts) in &self.harness.capability_mount_overrides {
            factory =
                factory.with_capability_execution_mount(capability_id.clone(), mounts.clone());
        }
        let port = factory.for_run_context(run_context.clone());
        // E-PROJ: see `apply_synthetic_capability_wrappers`'s doc comment.
        let port = self.harness.apply_synthetic_capability_wrappers(
            port,
            run_context,
            self.harness.io.clone(),
            result_writer,
        )?;
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
        request: ironclaw_turns::run_profile::RegisterProviderToolCallRequest,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        self.inner.register_provider_tool_call(request).await
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
    process_port: Option<Arc<dyn RuntimeProcessPort>>,
) -> HarnessResult<Arc<dyn HostRuntime>> {
    let mut registry = ExtensionRegistry::new();
    registry.insert(builtin_first_party_package()?)?;
    local_dev_host_runtime_with_registry_and_runtime_http_egress(
        storage_root,
        registry,
        egress,
        process_port,
    )
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
    process_port: Option<Arc<dyn RuntimeProcessPort>>,
) -> HarnessResult<Arc<dyn HostRuntime>> {
    let mut services = HostRuntimeServices::new(
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
    // Inject the recording process port when provided (slice 5). When `None`,
    // `HostRuntimeServices` defaults to `LocalHostProcessPort` (real execution).
    if let Some(port) = process_port {
        services = services.with_runtime_process_port_dyn(port);
    }

    Ok(Arc::new(services.host_runtime_for_local_testing()))
}

fn local_dev_host_runtime_with_registry_and_egress(
    storage_root: PathBuf,
    registry: ExtensionRegistry,
    runtime_http_egress: Arc<RecordingRuntimeHttpEgress>,
    network_egress: Arc<RecordingNetworkHttpEgress>,
    // E-AUTHGATE: `Ok(handle)` resolves the credential account (capability
    // dispatches); `Err(AuthRequired)` raises a `BlockedAuth` gate at dispatch.
    credential_account_result: Result<SecretHandle, CredentialStageError>,
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
        result: credential_account_result,
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

pub(crate) fn local_dev_root_filesystem(
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
    if mounts.web_access_assets {
        local.mount_local(
            VirtualPath::new("/system/extensions/web-access")?,
            HostPath::from_path_buf(harness_web_access::asset_root()),
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
    if mounts.web_access_assets {
        root.mount(
            local_dev_mount_descriptor(
                "/system/extensions/web-access",
                "local-dev-web-access-assets",
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
pub(crate) struct LocalDevRootMounts {
    github_assets: bool,
    web_access_assets: bool,
    memory: bool,
}

impl LocalDevRootMounts {
    pub(crate) fn core_builtins() -> Self {
        Self {
            github_assets: false,
            web_access_assets: false,
            memory: true,
        }
    }

    fn github_assets() -> Self {
        Self {
            github_assets: true,
            web_access_assets: false,
            memory: false,
        }
    }

    pub(crate) fn web_access_assets() -> Self {
        Self {
            github_assets: false,
            web_access_assets: true,
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
                EffectKind::DeleteFilesystem,
                EffectKind::Network,
                EffectKind::SpawnProcess,
                EffectKind::ExecuteCode,
                EffectKind::ExternalWrite,
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

fn wildcard_test_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: vec![NetworkTargetPattern {
            scheme: None,
            host_pattern: "*".to_string(),
            port: None,
        }],
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(1_000_000),
    }
}

/// C-JOURNEY: recursively copy `src` into `dst` (creating `dst` and any
/// intermediate directories). Used to populate a harness's per-test
/// `/system/extensions/<id>` mount with the real bundled-extension asset
/// directory (manifest + wasm module + schemas) so a WASM capability
/// published via `publish_bundled_extension_for_test` is genuinely loadable,
/// not just registered as metadata.
fn copy_dir_recursive(src: &Path, dst: &Path) -> HarnessResult<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path)?;
        } else {
            std::fs::copy(entry.path(), &dst_path)?;
        }
    }
    Ok(())
}

fn capability_ids_from_strs(ids: &[&str]) -> HarnessResult<Vec<CapabilityId>> {
    ids.iter()
        .map(|id| CapabilityId::new(*id).map_err(Into::into))
        .collect()
}

fn bundled_extension_provider_trust() -> HarnessResult<Vec<(ExtensionId, Vec<EffectKind>)>> {
    BUNDLED_EXTENSION_IDS
        .iter()
        .map(|id| Ok((ExtensionId::new(*id)?, local_dev_all_effects())))
        .collect()
}

fn local_dev_all_effects() -> Vec<EffectKind> {
    vec![
        EffectKind::DispatchCapability,
        EffectKind::ReadFilesystem,
        EffectKind::WriteFilesystem,
        EffectKind::DeleteFilesystem,
        EffectKind::Network,
        EffectKind::UseSecret,
        EffectKind::SpawnProcess,
        EffectKind::ExecuteCode,
        EffectKind::ExternalWrite,
    ]
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
        _expires_at: Option<ironclaw_host_api::Timestamp>,
    ) -> Result<SecretMetadata, SecretStoreError> {
        Ok(SecretMetadata {
            scope,
            handle,
            expires_at: None,
        })
    }

    async fn metadata(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMetadata>, SecretStoreError> {
        Ok((handle == &self.handle).then(|| SecretMetadata {
            scope: scope.clone(),
            handle: handle.clone(),
            expires_at: None,
        }))
    }

    async fn metadata_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretMetadata>, SecretStoreError> {
        Ok(vec![SecretMetadata {
            scope: scope.clone(),
            handle: self.handle.clone(),
            expires_at: None,
        }])
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
pub(crate) struct RecordingRuntimeHttpEgress {
    default_body: Vec<u8>,
    /// URL/method/capability-keyed scripted responses (§3.6 P1 ergonomics).
    /// Consulted before the FIFO queue; first match wins.
    scripted: Arc<Mutex<Vec<super::http_matcher::ScriptedHttpResponse>>>,
    response_bodies: Arc<Mutex<VecDeque<Vec<u8>>>>,
    requests: Arc<Mutex<Vec<RuntimeHttpEgressRequest>>>,
}

impl RecordingRuntimeHttpEgress {
    fn with_body(body: Vec<u8>) -> Self {
        Self {
            default_body: body,
            scripted: Arc::new(Mutex::new(Vec::new())),
            response_bodies: Arc::new(Mutex::new(VecDeque::new())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<RuntimeHttpEgressRequest> {
        self.requests.lock().unwrap().clone()
    }

    /// Append keyed scripted responses (the canonical keyed-matcher install).
    fn install_scripted(
        &self,
        responses: impl IntoIterator<Item = super::http_matcher::ScriptedHttpResponse>,
    ) {
        self.scripted.lock().unwrap().extend(responses);
    }

    /// Enqueue one FIFO response body (C-WEBACCESS), consumed in call order
    /// ahead of `default_body`. Mirrors `install_scripted`'s shape but for the
    /// plain FIFO queue rather than keyed matchers — used to script the
    /// three-leg Exa MCP handshake (`initialize` → `notifications/initialized`
    /// → `tools/call`), which all target the same URL/method/capability and so
    /// cannot be told apart by the keyed matcher.
    pub(crate) fn push_response_body(&self, body: Vec<u8>) {
        self.response_bodies.lock().unwrap().push_back(body);
    }
}

#[async_trait::async_trait]
impl RuntimeHttpEgress for RecordingRuntimeHttpEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        let request_bytes = request.body.len() as u64;
        // Resolve the keyed outcome BEFORE recording the request: `push(request)`
        // moves `request` by value into the log, so any code reading its fields
        // (the `.matches()` lookup) must run first. (`RuntimeHttpEgressRequest`
        // does implement `Drop`/`ZeroizeOnDrop` to scrub its URL/headers, but that
        // fires later when the logged entry is actually dropped, not on push.)
        let keyed_outcome = {
            let scripted = self.scripted.lock().unwrap();
            scripted
                .iter()
                .find(|response| response.matches(&request))
                .map(|response| response.outcome())
        };
        self.requests.lock().unwrap().push(request);
        // A scripted egress error short-circuits with `Err`, driving the tool's
        // error mapping. A body outcome (or the FIFO/default fallback) returns
        // `Ok` with the scripted status/body.
        let (status, body) = match keyed_outcome {
            Some(super::http_matcher::ScriptedHttpOutcome::Error(error)) => return Err(error),
            Some(super::http_matcher::ScriptedHttpOutcome::Body { status, bytes }) => {
                (status, bytes)
            }
            None => (
                200,
                self.response_bodies
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or_else(|| self.default_body.clone()),
            ),
        };
        Ok(RuntimeHttpEgressResponse {
            status,
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
    ) -> Result<CapabilityWriteResult, AgentLoopHostError> {
        let capability_id = write.capability_id.clone();
        let output = write.output.clone();
        let write_result = self.inner.write_capability_result(write).await?;
        self.results.lock().unwrap().push(RecordedCapabilityResult {
            capability_id,
            output,
        });
        Ok(write_result)
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

fn qa_smoke_mounts() -> HarnessResult<MountView> {
    Ok(MountView::new(vec![
        MountGrant::new(
            MountAlias::new("/workspace")?,
            VirtualPath::new("/projects/workspace")?,
            MountPermissions::read_write_list_delete(),
        ),
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
            output_digest: None,
        })
    }
}

#[async_trait]
impl LoopCapabilityPort for RecordingTestCapabilityPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        let definitions = vec![ProviderToolDefinition {
            capability_id: self.primary_capability_id(),
            name: ProviderToolName::new(self.primary_tool_name()).expect("provider tool name"),
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
        request: ironclaw_turns::run_profile::RegisterProviderToolCallRequest,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        let call = request.tool_call;
        let capability_id = self.primary_capability_id();
        Ok(CapabilityCallCandidate {
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
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
            callable_capability_ids: None,
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

pub(crate) struct HarnessCapabilityPortFactory {
    pub(crate) port: Arc<RecordingTestCapabilityPort>,
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

pub(crate) struct StaticCapabilitySurfaceProfileResolver {
    pub(crate) allow_set: CapabilityAllowSet,
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

pub(crate) struct EmptyIdentityContextSource;

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

pub(crate) fn scoped_turns_fs(
    backend: Arc<HarnessTurnStorageBackend>,
    binding: &ResolvedBinding,
) -> HarnessResult<Arc<ScopedFilesystem<HarnessTurnBackend>>> {
    // Include agent_id and project_id in the path when present so that
    // distinct agents or projects stored under the same tenant/user
    // (e.g. shared-storage multi-harness tests) get isolated turn state
    // files and cannot cross-claim each other's queued runs.
    // The 4-arm match lives in `super::filesystem::turns_scope_path`; the
    // integration tier reuses it with a different prefix via
    // `scoped_turns_fs_composite` in builder.rs.
    let target = super::filesystem::turns_scope_path("/engine", binding);
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/turns").expect("valid turns alias"),
        VirtualPath::new(target).expect("valid turns target"),
        MountPermissions::read_write_list_delete(),
    )])?;
    Ok(Arc::new(ScopedFilesystem::with_fixed_view(
        turn_state_root_filesystem(backend)?,
        mounts,
    )))
}

fn turn_state_root_filesystem(
    backend: Arc<HarnessTurnStorageBackend>,
) -> HarnessResult<Arc<HarnessTurnBackend>> {
    let mut root = CompositeRootFilesystem::new();
    root.mount(
        local_dev_mount_descriptor(
            "/engine",
            "reborn-harness-turn-state",
            BackendKind::MemoryDocuments,
            StorageClass::StructuredRecords,
            ContentKind::StructuredRecord,
            IndexPolicy::NotIndexed,
            backend.capabilities(),
        )?,
        backend,
    )?;
    Ok(Arc::new(root))
}

pub fn trace_tool_call_response() -> ironclaw_loop_support::HostManagedModelResponse {
    ironclaw_loop_support::HostManagedModelResponse {
        safe_text_deltas: Vec::new(),
        safe_reasoning_deltas: Vec::new(),
        usage: None,
        output: ParentLoopOutput::CapabilityCalls(vec![CapabilityCallCandidate {
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
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
                provider_tool_name: ProviderToolName::new("test_echo").expect("provider tool name"),
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
