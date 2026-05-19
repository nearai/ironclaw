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
//! - external network, delivery, OAuth, and sandbox process execution are not
//!   exercised by this harness.

#![allow(dead_code)] // Shared by staged Reborn binary-E2E validation ports.

use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use ironclaw_filesystem::{LocalFilesystem, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{
    AgentId, CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind,
    ExtensionId, GrantConstraints, MountAlias, MountGrant, MountPermissions, MountView,
    NetworkPolicy, Principal, ProjectId, ResourceScope, RuntimeKind, TenantId, ThreadId,
    TrustClass, UserId, VirtualPath,
};
use ironclaw_host_runtime::{
    BUILTIN_FIRST_PARTY_PROVIDER, CapabilitySurfacePolicy, HostRuntime, READ_FILE_CAPABILITY_ID,
    SurfaceKind, WRITE_FILE_CAPABILITY_ID,
};
use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileResolver,
    HostIdentityContextBuildError, HostIdentityContextCandidate, HostIdentityContextSource,
    HostManagedModelRequest, HostRuntimeLoopCapabilityPortFactory,
};
use ironclaw_product_adapters::{ProductInboundAck, ProductWorkflow};
use ironclaw_product_workflow::{
    ConversationBindingService, DefaultInboundTurnService, DefaultProductWorkflow,
    IdempotencyLedger, InboundTurnService, ResolveBindingRequest, ResolvedBinding,
};
use ironclaw_reborn::{
    loop_driver_host::LoopCapabilityPortFactory,
    loop_exit_applier::{
        BlockedEvidenceRequest, CompletionEvidenceRequest, FailureEvidenceRequest,
        FinalCheckpointEvidenceRequest, LoopExitEvidencePort, ThreadCheckpointLoopExitEvidencePort,
    },
    runtime::{
        DefaultPlannedRuntimeConfig, DefaultPlannedRuntimeParts, RebornRuntimeLoopComposition,
        build_default_planned_runtime,
    },
    turn_runner::{TurnRunnerWakeSender, TurnRunnerWorker, TurnRunnerWorkerConfig},
};
use ironclaw_reborn_composition::{
    ProductLiveCapabilityIo, ProductLiveVisibleCapabilityRequestConfig, RebornBuildInput,
    build_reborn_services, visible_capability_request_for_run,
};
use ironclaw_threads::{
    FilesystemSessionThreadService, SessionThreadService, ThreadHistoryRequest,
    ThreadMessageRecord, ThreadScope,
};
use ironclaw_trust::EffectiveTrustClass;
use ironclaw_turns::{
    DefaultTurnCoordinator, FilesystemTurnStateStore, GateRef, GetLoopCheckpointRequest,
    GetRunStateRequest, IdempotencyKey, InMemoryCheckpointStateStore, LoopBlockedKind,
    LoopCheckpointKind, LoopCheckpointStore, LoopGateRef, ReplyTargetBindingRef, ResumeTurnRequest,
    SourceBindingRef, TurnActor, TurnCoordinator, TurnError, TurnRunId, TurnRunState, TurnScope,
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
use serde_json::json;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::{
    config::WaitConfig,
    filesystem::local_filesystem,
    model_replay::RebornTraceReplayModelGateway,
    product_workflow::{RebornProductWorkflowHarness, resource_scope},
    session_thread::RebornThreadHarness,
    test_adapter::{RebornTestIngress, RebornTestProductAdapter},
};

pub type HarnessWaitConfig = WaitConfig;

const TEST_CAPABILITY_ID: &str = "test.echo";
const TEST_CAPABILITY_SURFACE_VERSION: &str = "trace_replay_v1";

type HarnessResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
type HarnessCapabilityParts = (
    Arc<dyn LoopCapabilityPortFactory>,
    Arc<dyn CapabilitySurfaceProfileResolver>,
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
    coordinator: Arc<DefaultTurnCoordinator<FilesystemTurnStateStore<LocalFilesystem>>>,
    _product_harness: RebornProductWorkflowHarness,
    thread_harness: RebornThreadHarness,
    model_gateway: RebornTraceReplayModelGateway,
    capability_recorder: HarnessCapabilityRecorder,
    milestone_sink: Arc<ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink>,
    worker: Arc<TurnRunnerWorker>,
    cancel: CancellationToken,
    worker_task: Option<JoinHandle<()>>,
    _turn_root: Arc<tempfile::TempDir>,
    _wake_sender: TurnRunnerWakeSender,
}

pub struct SubmittedTurn {
    pub ack: ProductInboundAck,
    pub run_id: TurnRunId,
    pub thread_id: ThreadId,
    pub scope: TurnScope,
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

    pub async fn with_host_runtime_file_capabilities(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
    ) -> HarnessResult<Self> {
        let host_runtime = Arc::new(HostRuntimeCapabilityHarness::file_tools().await?);
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
        Self::with_model_gateway_capability_mode(
            conversation_id,
            model_gateway,
            HarnessCapabilityMode::Recording(capability_port),
            accept_harness_blocked_evidence,
        )
        .await
    }

    async fn with_model_gateway_capability_mode(
        conversation_id: &str,
        model_gateway: RebornTraceReplayModelGateway,
        capability_mode: HarnessCapabilityMode,
        accept_harness_blocked_evidence: bool,
    ) -> HarnessResult<Self> {
        let adapter = RebornTestProductAdapter::new("reborn-test", "install-1")?;
        let ingress = RebornTestIngress::new(adapter);
        let product_harness = RebornProductWorkflowHarness::filesystem_temp(product_scope())?;
        let binding = product_harness
            .binding_service()?
            .resolve_binding(binding_request(&ingress, conversation_id)?)
            .await?;
        let thread_scope = thread_scope_from_binding(&binding)?;
        let turn_scope = TurnScope::new(
            binding.tenant_id.clone(),
            binding.agent_id.clone(),
            binding.project_id.clone(),
            binding.thread_id.clone(),
        );
        let thread_harness = RebornThreadHarness::filesystem_temp(thread_scope.clone())?;
        let turn_root = Arc::new(tempfile::tempdir()?);
        let turn_store = Arc::new(FilesystemTurnStateStore::new(scoped_turns_fs(
            Arc::new(local_filesystem(turn_root.path())?),
            &binding,
        )?));
        let checkpoint_state_store = Arc::new(InMemoryCheckpointStateStore::default());
        let loop_checkpoint_store: Arc<dyn LoopCheckpointStore> = turn_store.clone();
        let milestone_sink =
            Arc::new(ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink::default());
        let (capability_factory, capability_surface_resolver, capability_recorder) =
            capability_mode.into_parts(milestone_sink.clone())?;
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
        let composition = build_default_planned_runtime(DefaultPlannedRuntimeParts {
            turn_state: Arc::clone(&turn_store),
            thread_service: thread_harness.service.clone(),
            thread_scope: thread_scope.clone(),
            model_gateway: Arc::new(model_gateway.clone()),
            checkpoint_state_store,
            loop_checkpoint_store,
            milestone_sink: milestone_sink.clone(),
            capability_factory,
            capability_surface_resolver,
            loop_exit_evidence: evidence,
            config: DefaultPlannedRuntimeConfig {
                worker: TurnRunnerWorkerConfig {
                    heartbeat_interval: Duration::from_millis(20),
                    poll_interval: Duration::from_millis(10),
                    scope_filter: Some(turn_scope.clone()),
                },
                ..DefaultPlannedRuntimeConfig::default()
            },
            model_route_resolver: None,
            cancellation_factory: None,
            skill_context_source: None,
            input_queue: None,
            identity_context_source: Arc::new(EmptyIdentityContextSource),
            model_policy_guard: None,
            model_budget_accountant: None,
            safety_context: None,
        })?;
        let inbound: Arc<dyn InboundTurnService> = Arc::new(DefaultInboundTurnService::new(
            product_harness.binding_service()?,
            thread_harness.service_instance()?,
            composition.coordinator.clone(),
        ));
        let ledger: Arc<dyn IdempotencyLedger> = Arc::new(product_harness.idempotency_ledger());
        let workflow = DefaultProductWorkflow::new(inbound, ledger);

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
            FilesystemTurnStateStore<LocalFilesystem>,
            FilesystemSessionThreadService<LocalFilesystem>,
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
            worker_task: None,
            _turn_root: turn_root,
            _wake_sender: composition.wake_sender,
        }
    }

    pub fn start(&mut self) {
        if self.worker_task.is_some() {
            return;
        }
        let worker = Arc::clone(&self.worker);
        let cancel = self.cancel.clone();
        self.worker_task = Some(tokio::spawn(async move {
            worker.run(cancel).await;
        }));
    }

    pub async fn shutdown(&mut self) {
        self.cancel.cancel();
        if let Some(task) = self.worker_task.take() {
            let _ = task.await;
        }
    }

    pub async fn submit_text(&self, event_id: &str, text: &str) -> HarnessResult<SubmittedTurn> {
        let envelope = self.ingress.verified_text_envelope(
            event_id,
            "alice",
            &self.external_conversation_id,
            text,
        )?;
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
            thread_id: self.binding.thread_id.clone(),
            scope: self.turn_scope.clone(),
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

    pub async fn resume_with_gate(
        &self,
        run_id: TurnRunId,
        gate_ref: GateRef,
    ) -> HarnessResult<()> {
        let response = self
            .coordinator
            .resume_turn(ResumeTurnRequest {
                scope: self.turn_scope.clone(),
                actor: TurnActor::new(self.binding.user_id.clone()),
                run_id,
                gate_resolution_ref: gate_ref,
                source_binding_ref: SourceBindingRef::new("src:resume")?,
                reply_target_binding_ref: ReplyTargetBindingRef::new("reply:resume")?,
                idempotency_key: IdempotencyKey::new(format!("resume-{run_id}"))?,
            })
            .await?;
        if response.status != TurnStatus::Queued {
            return Err(format!("expected resumed run to queue, got {:?}", response.status).into());
        }
        Ok(())
    }

    pub async fn wait_for_status(
        &self,
        run_id: TurnRunId,
        expected: TurnStatus,
    ) -> HarnessResult<TurnRunState> {
        let wait = WaitConfig::default();
        let deadline = tokio::time::Instant::now() + wait.timeout;
        loop {
            let state = self.run_state(run_id).await?;
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
        Ok(self
            .turn_store
            .get_run_state(GetRunStateRequest {
                scope: self.turn_scope.clone(),
                run_id,
            })
            .await?)
    }

    pub async fn assert_final_reply(&self, text: &str) -> HarnessResult<()> {
        Ok(self
            .thread_harness
            .assert_final_reply(self.binding.thread_id.clone(), text)
            .await?)
    }

    pub async fn history(&self) -> HarnessResult<Vec<ThreadMessageRecord>> {
        Ok(self
            .thread_harness
            .service
            .list_thread_history(ThreadHistoryRequest {
                scope: self.thread_scope.clone(),
                thread_id: self.binding.thread_id.clone(),
            })
            .await?
            .messages)
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
        if request.blocked.kind != LoopBlockedKind::Approval
            || GateRef::new(request.blocked.gate_ref.as_str()).is_err()
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
                Ok((
                    Arc::new(HarnessCapabilityPortFactory {
                        port: Arc::clone(&port),
                    }),
                    Arc::new(StaticCapabilitySurfaceProfileResolver {
                        allow_set: CapabilityAllowSet::allowlist([CapabilityId::new(
                            TEST_CAPABILITY_ID,
                        )?]),
                    }),
                    HarnessCapabilityRecorder::Recording(port),
                ))
            }
            Self::HostRuntime(harness) => Ok((
                harness.capability_factory(milestone_sink),
                Arc::new(StaticCapabilitySurfaceProfileResolver {
                    allow_set: CapabilityAllowSet::allowlist(harness.capability_ids.clone()),
                }),
                HarnessCapabilityRecorder::HostRuntime(harness),
            )),
        }
    }
}

struct HostRuntimeCapabilityHarness {
    runtime: Arc<dyn HostRuntime>,
    io: Arc<ProductLiveCapabilityIo>,
    root: Arc<tempfile::TempDir>,
    workspace_root: PathBuf,
    mounts: MountView,
    capability_ids: Vec<CapabilityId>,
    effect_kinds: Vec<EffectKind>,
    user_id: UserId,
    invocations: Arc<Mutex<Vec<CapabilityInvocation>>>,
}

impl HostRuntimeCapabilityHarness {
    async fn file_tools() -> HarnessResult<Self> {
        let root = Arc::new(tempfile::tempdir()?);
        let storage_root = root.path().join("local-dev");
        let workspace_root = storage_root.join("workspace");
        std::fs::create_dir_all(&workspace_root)?;
        let services = build_reborn_services(RebornBuildInput::local_dev(
            "reborn-e2e-builtin-tools",
            storage_root,
        ))
        .await?;
        let runtime = services
            .host_runtime
            .ok_or("local-dev Reborn services missing host runtime")?;
        let mounts = workspace_mounts(MountPermissions::read_write_list_delete())?;
        Ok(Self {
            runtime,
            io: Arc::new(ProductLiveCapabilityIo::default()),
            root,
            workspace_root,
            mounts,
            capability_ids: vec![
                CapabilityId::new(WRITE_FILE_CAPABILITY_ID)?,
                CapabilityId::new(READ_FILE_CAPABILITY_ID)?,
            ],
            effect_kinds: vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
            user_id: UserId::new("reborn-e2e-builtin-user")?,
            invocations: Arc::new(Mutex::new(Vec::new())),
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

    fn invocations(&self) -> Vec<CapabilityInvocation> {
        self.invocations.lock().unwrap().clone()
    }

    fn workspace_file_path(&self, relative: &str) -> PathBuf {
        self.workspace_root.join(relative.trim_start_matches('/'))
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
            RuntimeKind::FirstParty,
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
        ))
        .with_provider_trust_for_effects(
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER).map_err(host_runtime_harness_error)?,
            EffectiveTrustClass::user_trusted(),
            self.harness.effect_kinds.clone(),
        );
        let execution_mounts = self.harness.mounts.clone();
        let visible_request = visible_capability_request_for_run(run_context, authority)
            .map_err(host_runtime_harness_error)?;
        let milestone_sink: Arc<dyn LoopHostMilestoneSink> = self.milestone_sink.clone();
        let port = HostRuntimeLoopCapabilityPortFactory::new(
            Arc::clone(&self.harness.runtime),
            visible_request,
            self.harness.io.clone(),
            self.harness.io.clone(),
            Some(milestone_sink),
        )
        .with_execution_mounts(execution_mounts)
        .for_run_context(run_context.clone());
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

fn workspace_mounts(permissions: MountPermissions) -> HarnessResult<MountView> {
    Ok(MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace")?,
        VirtualPath::new("/projects/workspace")?,
        permissions,
    )])?)
}

fn capability_grants(
    grantee: Principal,
    capabilities: &[CapabilityId],
    allowed_effects: Vec<EffectKind>,
    mounts: MountView,
) -> CapabilitySet {
    CapabilitySet {
        grants: capabilities
            .iter()
            .map(|capability| CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: capability.clone(),
                grantee: grantee.clone(),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: allowed_effects.clone(),
                    mounts: mounts.clone(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
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
    invocations: Arc<Mutex<Vec<CapabilityInvocation>>>,
    next_result: Arc<AtomicUsize>,
    approval_calls: Arc<AtomicUsize>,
}

#[derive(Debug, Clone, Copy)]
enum CapabilityMode {
    Echo,
    ApprovalThenEcho,
}

impl RecordingTestCapabilityPort {
    pub fn echo() -> Self {
        Self::new(CapabilityMode::Echo)
    }

    pub fn approval_then_echo() -> Self {
        Self::new(CapabilityMode::ApprovalThenEcho)
    }

    fn new(mode: CapabilityMode) -> Self {
        Self {
            mode,
            invocations: Arc::new(Mutex::new(Vec::new())),
            next_result: Arc::new(AtomicUsize::new(1)),
            approval_calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn invocations(&self) -> Vec<CapabilityInvocation> {
        self.invocations.lock().unwrap().clone()
    }

    pub fn invocation_count(&self) -> usize {
        self.invocations.lock().unwrap().len()
    }

    fn completed_result(&self) -> CapabilityOutcome {
        let ordinal = self.next_result.fetch_add(1, Ordering::SeqCst);
        CapabilityOutcome::Completed(CapabilityResultMessage {
            result_ref: ironclaw_turns::LoopResultRef::new(format!("result:test-echo-{ordinal}"))
                .expect("valid result ref"),
            safe_summary: "echo: hi".to_string(),
            terminate_hint: false,
        })
    }
}

#[async_trait]
impl LoopCapabilityPort for RecordingTestCapabilityPort {
    fn tool_definitions(&self) -> Result<Vec<ProviderToolDefinition>, AgentLoopHostError> {
        Ok(vec![ProviderToolDefinition {
            capability_id: CapabilityId::new(TEST_CAPABILITY_ID).expect("valid capability id"),
            name: "test_echo".to_string(),
            description: "Echo a test payload".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "message": {"type": "string"}
                }
            }),
        }])
    }

    async fn register_provider_tool_call(
        &self,
        call: ProviderToolCall,
    ) -> Result<CapabilityCallCandidate, AgentLoopHostError> {
        Ok(CapabilityCallCandidate {
            surface_version: CapabilitySurfaceVersion::new(TEST_CAPABILITY_SURFACE_VERSION)
                .expect("valid surface version"),
            capability_id: CapabilityId::new(TEST_CAPABILITY_ID).expect("valid capability id"),
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
        Ok(VisibleCapabilitySurface {
            version: CapabilitySurfaceVersion::new(TEST_CAPABILITY_SURFACE_VERSION)
                .expect("valid surface version"),
            descriptors: vec![CapabilityDescriptorView {
                capability_id: CapabilityId::new(TEST_CAPABILITY_ID).expect("valid capability id"),
                provider: Some(ExtensionId::new("test").expect("valid provider")),
                runtime: RuntimeKind::FirstParty,
                safe_name: "test_echo".to_string(),
                safe_description: "Echo a test payload".to_string(),
                concurrency_hint: ConcurrencyHint::SafeForParallel,
                parameters_schema: json!({"type": "object"}),
            }],
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
            });
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
    resource_scope(
        TenantId::new("tenant-e2e").expect("valid tenant"),
        UserId::new("host-user").expect("valid user"),
        AgentId::new("agent-e2e").expect("valid agent"),
        Some(ProjectId::new("project-e2e").expect("valid project")),
    )
}

fn binding_request(
    ingress: &RebornTestIngress,
    conversation_id: &str,
) -> HarnessResult<ResolveBindingRequest> {
    let envelope =
        ingress.verified_text_envelope("binding-probe", "alice", conversation_id, "hi")?;
    Ok(ResolveBindingRequest {
        adapter_id: envelope.adapter_id().clone(),
        installation_id: envelope.installation_id().clone(),
        external_actor_ref: envelope.external_actor_ref().clone(),
        external_conversation_ref: envelope.external_conversation_ref().clone(),
        auth_claim: envelope.auth_claim().clone(),
    })
}

fn thread_scope_from_binding(binding: &ResolvedBinding) -> HarnessResult<ThreadScope> {
    Ok(ThreadScope {
        tenant_id: binding.tenant_id.clone(),
        agent_id: binding
            .agent_id
            .clone()
            .ok_or("resolved binding missing agent id")?,
        project_id: binding.project_id.clone(),
        owner_user_id: Some(binding.user_id.clone()),
        mission_id: None,
    })
}

fn scoped_turns_fs<F>(
    backend: Arc<F>,
    binding: &ResolvedBinding,
) -> HarnessResult<Arc<ScopedFilesystem<F>>>
where
    F: RootFilesystem,
{
    let target = format!(
        "/engine/tenants/{}/users/{}/turns",
        binding.tenant_id, binding.user_id
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
        output: ParentLoopOutput::CapabilityCalls(vec![CapabilityCallCandidate {
            surface_version: CapabilitySurfaceVersion::new(TEST_CAPABILITY_SURFACE_VERSION)
                .expect("valid surface version"),
            capability_id: CapabilityId::new(TEST_CAPABILITY_ID).expect("valid capability id"),
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
