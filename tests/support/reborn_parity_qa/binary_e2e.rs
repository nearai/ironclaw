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

use std::{path::PathBuf, sync::Arc, time::Duration};

use async_trait::async_trait;
use ironclaw_filesystem::{DiskFilesystem, InMemoryBackend};
use ironclaw_host_api::{
    CapabilityId, NetworkPolicy, ProviderToolName, ResourceScope, RuntimeHttpEgressRequest,
    ThreadId,
};
use ironclaw_loop_host::{
    EmptyUserProfileSource, HostIdentityContextSource, HostManagedModelRequest,
    JsonSpawnSubagentInputCodec,
};
use ironclaw_network::NetworkHttpRequest;
use ironclaw_product_adapters::{
    ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload, ProductTriggerReason,
    ProductWorkflow,
};
use ironclaw_product_workflow::{
    ConversationBindingService, DefaultInboundTurnService, DefaultProductWorkflow,
    IdempotencyLedger, InboundTurnService, ProductConversationRouteKind, ResolveBindingRequest,
    ResolvedBinding,
};
use ironclaw_runner::subagent::{
    await_edge::{
        boot_recovery::ScopeRecoveryDriver, resolver::AwaitEdgeResolver,
        store::FilesystemAwaitEdgeStore,
    },
    flavors::StaticSubagentDefinitionResolver,
    goal_store::InMemoryBoundedSubagentGoalStore,
};
use ironclaw_runner::turn_scheduler::{SchedulerTurnRunWakeNotifier, TurnRunSchedulerHandle};
use ironclaw_runner::{
    loop_exit_applier::{
        BlockedEvidenceRequest, CompletionEvidenceRequest, FailureEvidenceRequest,
        FinalCheckpointEvidenceRequest, LoopExitEvidencePort, ThreadCheckpointLoopExitEvidencePort,
    },
    runtime::{
        DefaultPlannedRuntimeConfig, DefaultPlannedRuntimeParts, RebornRuntimeLoopComposition,
        RuntimeTurnStateStore, build_default_planned_runtime,
    },
};
use ironclaw_threads::{
    FilesystemSessionThreadService, SessionThreadService, ThreadHistoryRequest,
    ThreadMessageRecord, ThreadScope,
};
use ironclaw_turns::{
    CancelRunRequest, FilesystemTurnStateStore, GateRef, GetLoopCheckpointRequest,
    GetRunStateRequest, IdempotencyKey, InMemoryCheckpointStateStore, LoopBlockedKind,
    LoopCheckpointKind, LoopCheckpointStore, ReplyTargetBindingRef, ResumeTurnRequest,
    RetryTurnRequest, RetryTurnResponse, SanitizedCancelReason, SourceBindingRef, TurnActor,
    TurnCoordinator, TurnError, TurnRunId, TurnRunRecord, TurnRunState, TurnScope,
    TurnSpawnTreeStateStore, TurnStateStore, TurnStatus,
    run_profile::{
        CapabilityCallCandidate, CapabilityInputRef, CapabilityInvocation,
        CapabilitySurfaceVersion, LoopHostMilestone, LoopHostMilestoneKind, ParentLoopOutput,
        ProviderToolCallReplay,
    },
};
use serde_json::json;

use super::model_replay::RebornTraceReplayModelGateway;
use crate::reborn_support::config::WaitConfig;
use crate::reborn_support::doubles::{
    EmptyIdentityContextSource, RecordingTestCapabilityPort, TEST_CAPABILITY_ID,
    TEST_CAPABILITY_SURFACE_VERSION,
};
use crate::reborn_support::filesystem::{BlockingTurnStatePutFilesystem, local_filesystem};
use crate::reborn_support::harness::profiles::core_builtin::{self, CoreBuiltinOptions};
use crate::reborn_support::harness::{
    HarnessCapabilityMode, HarnessCapabilityRecorder, HarnessResult, HarnessTurnBackend,
    HarnessTurnStorageBackend, RecordedCapabilityResult, product_scope, scoped_turns_fs,
};
use crate::reborn_support::product_workflow::RebornProductWorkflowHarness;
use crate::reborn_support::session_thread::RebornThreadHarness;
use crate::reborn_support::test_adapter::{RebornTestIngress, RebornTestProductAdapter};

pub type HarnessWaitConfig = WaitConfig;

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
    product_backend: Arc<DiskFilesystem>,
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

impl RebornBinaryE2EHarness {
    pub async fn reply_only(
        conversation_id: &str,
        reply: impl Into<String>,
    ) -> HarnessResult<Self> {
        Self::with_model_gateway(
            conversation_id,
            RebornTraceReplayModelGateway::with_responses([
                ironclaw_loop_host::HostManagedModelResponse::assistant_reply(reply),
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
        let host_runtime =
            Arc::new(crate::reborn_support::harness::profiles::file::file_tools().await?);
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
        // The production capability port resolves the dispatch scope
        // owner-first from the turn's real binding subject (this harness
        // submits as the fixed `"alice"` actor, not the profile's default
        // `"reborn-e2e-builtin-user"`), so the disabled global auto-approve
        // setting must be seeded under that SAME resolved subject or the
        // gate never raises -- mirrors
        // `with_host_runtime_extension_lifecycle_capabilities`.
        let subject_user = Self::resolve_default_binding_subject_user(conversation_id).await?;
        let host_runtime = Arc::new(
            crate::reborn_support::harness::profiles::file::file_tools_requiring_approval_profile_for_user(
                subject_user.as_str(),
            )?
            .build()
            .await?,
        );
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
        let host_runtime =
            Arc::new(crate::reborn_support::harness::profiles::file::write_only().await?);
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
        let host_runtime = Arc::new(
            crate::reborn_support::harness::profiles::coding_read::coding_read_tools().await?,
        );
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
        let host_runtime = Arc::new(core_builtin::core_builtin_tools_default().await?);
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
        let host_runtime =
            Arc::new(crate::reborn_support::harness::profiles::process::process_tools().await?);
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
        let host_runtime =
            Arc::new(crate::reborn_support::harness::profiles::qa_smoke::qa_smoke_tools().await?);
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
        // Invariant (#5459): the profile (credential seeding included) must be
        // built under the same subject user the turn's authenticated binding
        // resolves to — extension_remove reads ownership under that actor, so a
        // fixed profile user makes install-then-remove see "never installed".
        // Mirrors `build_group_capability_with_base` in the group harness.
        let subject_user = Self::resolve_default_binding_subject_user(conversation_id).await?;
        let host_runtime = Arc::new(
            crate::reborn_support::harness::profiles::extension::extension_lifecycle_tools_profile_for_user(
                subject_user.as_str(),
            )?
            .build()
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

    /// Resolve the `(tenant, subject-user)` a default `submit_text` call
    /// (actor `"alice"`, adapter `"reborn-test"`, installation `"install-1"`)
    /// will bind to for `conversation_id`, WITHOUT depending on the harness
    /// under construction. Deterministic and side-effect-free from the
    /// caller's perspective (its own throwaway `filesystem_temp` product
    /// harness/backend): direct-chat routes set `subject_user_id` to the
    /// resolved actor (`ResolvedBinding` doc comment), so this reproduces
    /// exactly what the real turn's binding resolves to later.
    async fn resolve_default_binding_subject_user(
        conversation_id: &str,
    ) -> HarnessResult<ironclaw_host_api::UserId> {
        let adapter = RebornTestProductAdapter::new("reborn-test", "install-1")?;
        let ingress = RebornTestIngress::new(adapter);
        let envelope = ingress.verified_text_envelope_with_trigger(
            "extension-lifecycle-actor-probe",
            "alice",
            conversation_id,
            "probe",
            ProductTriggerReason::DirectChat,
        )?;
        let binding_request = binding_request_from_envelope(&envelope);
        let product_harness = RebornProductWorkflowHarness::filesystem_temp(product_scope())?;
        let binding = product_harness
            .binding_service()?
            .resolve_binding(binding_request)
            .await?;
        Ok(binding.subject_user_id.unwrap_or(binding.actor_user_id))
    }

    pub async fn with_host_runtime_skill_management_capabilities(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
    ) -> HarnessResult<Self> {
        let host_runtime = Arc::new(
            crate::reborn_support::harness::profiles::skill::skill_management_tools().await?,
        );
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
        let host_runtime = Arc::new(
            crate::reborn_support::harness::profiles::trigger::trigger_management_tools().await?,
        );
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
        let host_runtime = Arc::new(
            crate::reborn_support::harness::profiles::trace_commons::trace_commons_tools().await?,
        );
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
            core_builtin::core_builtin_tools(
                CoreBuiltinOptions::default().with_network_policy(network_policy),
            )
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
            core_builtin::core_builtin_tools(
                CoreBuiltinOptions::default()
                    .with_live_http_egress()
                    .with_network_policy(network_policy),
            )
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
        let host_runtime =
            Arc::new(crate::reborn_support::harness::profiles::github::github_issue_tools().await?);
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
        let turns_scoped_fs = scoped_turns_fs(turn_backend, &binding)?;
        let turn_store = Arc::new(FilesystemTurnStateStore::new(Arc::clone(&turns_scoped_fs)));
        let checkpoint_state_store = Arc::new(InMemoryCheckpointStateStore::default());
        let loop_checkpoint_store: Arc<dyn LoopCheckpointStore> = turn_store.clone();
        let milestone_sink =
            Arc::new(ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink::default());
        let exposes_spawn_subagent = capability_mode.exposes_spawn_subagent();
        let (
            capability_factory,
            capability_surface_resolver,
            capability_input_resolver,
            capability_result_writer,
            capability_recorder,
        ) = capability_mode.into_parts(
            milestone_sink.clone(),
            thread_harness.service.clone() as Arc<dyn SessionThreadService>,
            Arc::clone(&turn_store),
        )?;
        // Same shared `ScopedFilesystem` handle the turn store uses (`/turns`
        // mount) — the await-edge tree lives at
        // `/turns/subagent-await-edges/...`, a sibling prefix, per §4.5a's
        // "one shared handle, never a per-store fixed view" rule.
        let await_edge_store =
            Arc::new(FilesystemAwaitEdgeStore::new(Arc::clone(&turns_scoped_fs)));
        let await_edge_goal_store = Arc::new(InMemoryBoundedSubagentGoalStore::new());
        let await_edge_resolver = Arc::new(AwaitEdgeResolver::new_unbound(
            Arc::clone(&await_edge_store),
            await_edge_goal_store.clone() as Arc<dyn ironclaw_loop_host::SubagentSpawnGoalStore>,
            turn_store.clone() as Arc<dyn ironclaw_turns::TurnSpawnTreeStateStore>,
            capability_result_writer.clone(),
            thread_harness.service.clone(),
        ));
        let await_edge_driver = Arc::new(ScopeRecoveryDriver::new(
            Arc::clone(&await_edge_resolver),
            Arc::clone(&await_edge_store),
        ));
        let mut runtime_config = DefaultPlannedRuntimeConfig {
            // Keep the durable runner heartbeat at its production default;
            // test responsiveness comes from fast scheduler polling below.
            poll_interval: Duration::from_millis(10),
            // The binary-E2E harness runs many scripted runtimes in one test
            // process. Keep each harness deterministic; scheduler worker-pool
            // concurrency is covered by lower-level runtime tests.
            worker_count: Some(std::num::NonZeroUsize::MIN),
            // Scripted replay gateways fail deliberately (exhausted steps,
            // mismatched requests) and must reach Failed in seconds; the
            // production availability budget would ride those errors through
            // minutes of backoff. Mirrors the integration group harness's
            // IRONCLAW_REBORN_MODEL_AVAILABILITY_RETRY_ATTEMPTS=1 pin.
            planned_model_availability_retry_attempts: std::num::NonZeroU32::new(1),
            ..DefaultPlannedRuntimeConfig::default()
        };
        if exposes_spawn_subagent {
            // Explicit spawn regression tests need the tool surface even though
            // production currently disables model-facing spawn by default.
            runtime_config.disabled_capability_ids = Vec::new();
        }
        let turn_state_for_evidence: Arc<dyn TurnStateStore> = turn_store.clone();
        let evidence = Arc::new(HarnessLoopExitEvidencePort {
            inner: ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
                thread_harness.service.clone(),
                turn_state_for_evidence,
                Arc::clone(&loop_checkpoint_store),
                Arc::clone(&await_edge_store)
                    as Arc<dyn ironclaw_runner::loop_exit_applier::AwaitDependentRunEvidenceStore>,
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
            subagent_goal_store: await_edge_goal_store,
            subagent_await_edge_writer: await_edge_driver
                as Arc<dyn ironclaw_loop_host::AwaitEdgeWriter>,
            subagent_await_edge_settler: await_edge_resolver
                as Arc<dyn ironclaw_loop_host::AwaitEdgeSettler>,
            subagent_await_edge_evidence: await_edge_store
                as Arc<dyn ironclaw_runner::loop_exit_applier::AwaitDependentRunEvidenceStore>,
            subagent_definition_resolver: Arc::new(StaticSubagentDefinitionResolver),
            subagent_spawn_input_codec: Arc::new(JsonSpawnSubagentInputCodec::new(
                capability_input_resolver,
            )),
            subagent_spawn_limits: ironclaw_loop_host::SubagentSpawnLimits::default(),
            loop_exit_evidence: evidence,
            config: runtime_config,
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

    pub async fn retry_turn(&self, run_id: TurnRunId) -> HarnessResult<RetryTurnResponse> {
        Ok(self
            .coordinator
            .retry_turn(RetryTurnRequest {
                scope: self.turn_scope.clone(),
                actor: TurnActor::new(self.binding.actor_user_id.clone()),
                run_id,
                source_binding_ref: SourceBindingRef::new("src:retry")?,
                reply_target_binding_ref: ReplyTargetBindingRef::new("reply:retry")?,
                idempotency_key: IdempotencyKey::new(format!("retry-{run_id}"))?,
            })
            .await?)
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

pub fn trace_tool_call_response() -> ironclaw_loop_host::HostManagedModelResponse {
    ironclaw_loop_host::HostManagedModelResponse {
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
