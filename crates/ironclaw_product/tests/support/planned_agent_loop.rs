use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Display;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_approvals::AutoApproveSettingInput;
use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{
    AgentId, CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind,
    ExtensionId, GrantConstraints, InvocationId, MountAlias, MountGrant, MountPermissions,
    MountView, NetworkPolicy, Principal, Resolution, ResolutionBatch, ResourceScope, RuntimeKind,
    TenantId, ThreadId, TrustClass, UserId, VirtualPath,
};
use ironclaw_host_runtime::{CapabilitySurfacePolicy, SurfaceKind};
use ironclaw_loop_host::{
    CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileResolver,
    EmptyLoopCapabilityPort, EmptyUserProfileSource, HostIdentityContextBuildError,
    HostIdentityContextCandidate, HostIdentityContextSource, HostInputBatch, HostInputQueue,
    HostInputQueueError, HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelRequest, HostManagedModelResponse, JsonSpawnSubagentInputCodec,
    LoopCapabilityPortFactory, LoopCapabilityResultWriter, ProductLiveCancellationProbe,
    RunCancellationFactory, RunCancellationHandle,
};
use ironclaw_product::{
    AdapterInstallationId, AuthRequirement, ExternalActorRef, ExternalConversationRef,
    ExternalEventId, ParsedProductInbound, ProductAdapterId, ProductInboundEnvelope,
    ProductInboundPayload, ProductTriggerReason, ProtocolAuthEvidence, TrustedInboundContext,
    UserMessagePayload,
};
use ironclaw_product::{
    DefaultInboundTurnService, FakeConversationBindingService, InboundTurnOutcome,
    InboundTurnService, ResolvedBinding,
};
use ironclaw_reborn_composition::{
    ProductLiveCapabilityAuthorityResolver, ProductLiveCapabilityIo, ProductLiveModelRouteSettings,
    ProductLivePlannedRuntimeAdapterConfig, ProductLivePlannedRuntimeAdapterError,
    ProductLivePlannedRuntimeAdapters, ProductLiveVisibleCapabilityRequestConfig, RebornRuntime,
    RebornRuntimeInput, build_runtime, capability_allowlist,
};
use ironclaw_runner::{
    loop_exit_applier::ThreadCheckpointLoopExitEvidencePort,
    model_routes::{
        ModelRoute, ModelRoutePolicy, ModelSelectionMode, ModelSlot, StaticModelRouteResolver,
    },
    runtime::{
        DefaultPlannedRuntimeConfig, DefaultPlannedRuntimeParts, RebornRuntimeLoopComposition,
        RuntimeTurnStateStore, build_product_live_planned_runtime,
    },
    subagent::await_edge::{
        boot_recovery::ScopeRecoveryDriver, resolver::AwaitEdgeResolver,
        store::FilesystemAwaitEdgeStore,
    },
    subagent::goal_store::in_memory_backed_subagent_goal_store,
};
use ironclaw_threads::{
    InMemorySessionThreadService, SessionThreadService, ThreadHistoryRequest, ThreadMessageRecord,
    ThreadScope,
};
use ironclaw_trust::EffectiveTrustClass;
use ironclaw_turns::test_support::in_memory_turn_state_store;
use ironclaw_turns::{
    CancelRunRequest, FilesystemTurnStateRowStore, GetRunStateRequest, IdempotencyKey,
    LoopResultRef, SanitizedCancelReason, TurnActor, TurnCoordinator, TurnRunId, TurnRunState,
    TurnRunWake, TurnScope, TurnStateStore, TurnStatus,
    run_profile::{
        AgentLoopHostError, CapabilityCallCandidate, CapabilityDescriptorView, CapabilityInputRef,
        CapabilitySurfaceVersion, ConcurrencyHint, InMemoryLoopHostMilestoneSink,
        InstructionSafetyContext, LoopCancelReasonKind, LoopCapabilityPort, LoopInputAckToken,
        LoopInputCursorToken, LoopRequest, LoopRequestBatch, LoopRunContext, NoOpBudgetAccountant,
        NoOpPolicyGuard, ParentLoopOutput, PromptMode, VisibleCapabilityRequest,
        VisibleCapabilitySurface, resolution,
    },
};
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;

use ironclaw_loop_host::in_memory_backed_checkpoint_state_store as in_memory_checkpoint_state_store;

pub struct ProductLiveAgentLoopHarness {
    binding_service: FakeConversationBindingService,
    binding: ResolvedBinding,
    thread_scope: ThreadScope,
    thread_service: InMemorySessionThreadService,
    turn_store: Arc<FilesystemTurnStateRowStore<InMemoryBackend>>,
    cancellation_factory: Arc<ReadyRunCancellationFactory>,
    composition: RebornRuntimeLoopComposition<dyn SessionThreadService, RecordingModelGateway>,
    model_requests: Arc<Mutex<Vec<HostManagedModelRequest>>>,
    capability_invocations: Arc<Mutex<Vec<LoopRequest>>>,
    capability_results: Arc<Mutex<Vec<serde_json::Value>>>,
    model_release: Option<CancellationToken>,
    _host_runtime_root: Option<tempfile::TempDir>,
}

#[derive(Debug, Clone)]
pub struct ProductLiveAgentLoopHarnessConfig {
    pub assistant_reply: String,
    pub tenant_id: String,
    pub user_id: String,
    pub thread_id: String,
    pub agent_id: String,
    pub model_provider: String,
    pub model_id: String,
    pub pause_model_until_released: bool,
    pub model_responses: Vec<HostManagedModelResponse>,
    pub capability: Option<HarnessCapabilityConfig>,
    pub host_runtime_capability: Option<HostRuntimeCapabilityConfig>,
}

impl Default for ProductLiveAgentLoopHarnessConfig {
    fn default() -> Self {
        Self {
            assistant_reply: "planned harness reply".to_string(),
            tenant_id: "tenant:harness".to_string(),
            user_id: "user:harness".to_string(),
            thread_id: "thread:harness".to_string(),
            agent_id: "agent:harness".to_string(),
            model_provider: "nearai".to_string(),
            model_id: "qwen3-coder".to_string(),
            pause_model_until_released: false,
            model_responses: Vec::new(),
            capability: None,
            host_runtime_capability: None,
        }
    }
}

impl ProductLiveAgentLoopHarnessConfig {
    pub fn set_assistant_reply(mut self, assistant_reply: impl Into<String>) -> Self {
        self.assistant_reply = assistant_reply.into();
        self
    }

    pub fn set_user_id(mut self, user_id: impl Into<String>) -> Self {
        self.user_id = user_id.into();
        self
    }

    pub fn set_thread_id(mut self, thread_id: impl Into<String>) -> Self {
        self.thread_id = thread_id.into();
        self
    }

    pub fn set_pause_model_until_released(mut self, pause_model_until_released: bool) -> Self {
        self.pause_model_until_released = pause_model_until_released;
        self
    }

    pub fn set_model_responses(mut self, model_responses: Vec<HostManagedModelResponse>) -> Self {
        self.model_responses = model_responses;
        self
    }

    pub fn set_capability(mut self, capability: HarnessCapabilityConfig) -> Self {
        self.capability = Some(capability);
        self
    }

    pub fn set_host_runtime_capability(
        mut self,
        host_runtime_capability: HostRuntimeCapabilityConfig,
    ) -> Self {
        self.host_runtime_capability = Some(host_runtime_capability);
        self
    }
}

#[derive(Debug, Clone)]
pub struct HarnessCapabilityConfig {
    pub capability_id: String,
    pub result_ref: String,
    pub safe_summary: String,
    pub terminate_hint: bool,
}

#[derive(Debug, Clone)]
pub struct HostRuntimeCapabilityConfig {
    pub capability_id: String,
    pub input: serde_json::Value,
}

async fn enable_host_runtime_auto_approve_for_harness_user(
    services: &RebornRuntime,
    binding: &ResolvedBinding,
) {
    let auto_approve = services
        .local_dev_auto_approve_settings_for_test()
        .expect("local-dev host runtime auto-approve settings");
    let scope = ResourceScope {
        tenant_id: binding.tenant_id.clone(),
        user_id: binding
            .subject_user_id
            .clone()
            .expect("harness subject user id"),
        agent_id: binding.agent_id.clone(),
        project_id: binding.project_id.clone(),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    };
    auto_approve
        .set(AutoApproveSettingInput {
            updated_by: Principal::User(scope.user_id.clone()),
            scope,
            enabled: true,
        })
        .await
        .expect("enable host runtime auto-approve for harness user");
}

pub fn capability_call_response(
    capability_id: impl Into<String>,
    input_ref: impl Into<String>,
) -> HostManagedModelResponse {
    HostManagedModelResponse {
        safe_text_deltas: Vec::new(),
        safe_reasoning_deltas: Vec::new(),
        usage: None,
        output: ParentLoopOutput::CapabilityCalls(vec![CapabilityCallCandidate {
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
            surface_version: harness_surface_version(),
            capability_id: harness_capability_id(capability_id.into()),
            input_ref: CapabilityInputRef::new(input_ref.into()).expect("valid harness input ref"),
            effective_capability_ids: Vec::new(),
            provider_replay: None,
        }]),
    }
}

impl ProductLiveAgentLoopHarness {
    pub async fn new(config: ProductLiveAgentLoopHarnessConfig) -> Self {
        let binding_service = FakeConversationBindingService::new();
        let user_id = UserId::new(config.user_id).expect("valid harness user id");
        let binding = ResolvedBinding {
            tenant_id: TenantId::new(config.tenant_id).expect("valid harness tenant id"),
            actor_user_id: user_id.clone(),
            subject_user_id: Some(user_id),
            source_binding_ref: ironclaw_turns::SourceBindingRef::new("source:live-harness")
                .expect("valid harness source binding ref"),
            reply_target_binding_ref: ironclaw_turns::ReplyTargetBindingRef::new(
                "reply:live-harness",
            )
            .expect("valid harness reply target binding ref"),
            thread_id: ThreadId::new(config.thread_id).expect("valid harness thread id"),
            agent_id: Some(AgentId::new(config.agent_id).expect("valid harness agent id")),
            project_id: None,
        };
        let thread_scope = ThreadScope {
            tenant_id: binding.tenant_id.clone(),
            agent_id: binding.agent_id.clone().expect("harness agent id"),
            project_id: binding.project_id.clone(),
            owner_user_id: binding.subject_user_id.clone(),
            mission_id: None,
        };
        let thread_service = InMemorySessionThreadService::default();
        let turn_store = Arc::new(in_memory_turn_state_store());
        let checkpoint_store = Arc::clone(&turn_store);
        let model_requests = Arc::new(Mutex::new(Vec::new()));
        let model_responses = VecDeque::from(config.model_responses);
        let model_release = config
            .pause_model_until_released
            .then(CancellationToken::new);
        let host_runtime_root = config
            .host_runtime_capability
            .as_ref()
            .map(|_| tempfile::tempdir().expect("host runtime harness tempdir"));
        let host_runtime_services = if let Some(root) = &host_runtime_root {
            let services = build_runtime(RebornRuntimeInput::from_build_input(
                ironclaw_reborn_composition::local_dev_build_input(
                    "planned-harness-host-runtime",
                    root.path().join("local-dev"),
                ),
            ))
            .await
            .expect("host runtime harness services");
            enable_host_runtime_auto_approve_for_harness_user(&services, &binding).await;
            Some(Arc::new(services))
        } else {
            None
        };
        let host_runtime_io = config
            .host_runtime_capability
            .as_ref()
            .map(|_| Arc::new(ProductLiveCapabilityIo::default()));
        let host_runtime_staged_inputs = Arc::new(Mutex::new(HashMap::new()));
        let host_runtime_tool_call =
            config
                .host_runtime_capability
                .as_ref()
                .map(|capability| ScriptedHostRuntimeToolCall {
                    capability_id: harness_capability_id(&capability.capability_id),
                    staged_inputs: Arc::clone(&host_runtime_staged_inputs),
                    issued_runs: Arc::new(Mutex::new(HashSet::new())),
                });
        let model_gateway = Arc::new(RecordingModelGateway {
            reply: config.assistant_reply,
            requests: Arc::clone(&model_requests),
            responses: Mutex::new(model_responses),
            release: model_release.clone(),
            host_runtime_tool_call,
        });
        let cancellation_factory = Arc::new(ReadyRunCancellationFactory::default());
        let capability_invocations = Arc::new(Mutex::new(Vec::new()));
        let capability_results = Arc::new(Mutex::new(Vec::new()));
        // The durable gate-record store is shared between the ProductLive
        // capability port (which persists the record) and the turn executor
        // (which re-sources an auth block's credential requirements from it,
        // §5.2.9). Capture it here so the SAME store is wired into both — else the
        // executor gets `None` and an auth block is applied with empty
        // requirements / fails the exit (#6287 IronLoop). `None` for the
        // non-ProductLive fakes (which do not persist gate records), preserving
        // the executor's tolerant "no store wired" path.
        let mut turn_executor_gate_store: Option<Arc<dyn ironclaw_run_state::GateRecordStore>> =
            None;
        let capability_factory: Arc<dyn LoopCapabilityPortFactory> =
            if let Some(capability) = config.host_runtime_capability {
                // Durable gate-record + replay-payload stores over ONE in-memory
                // filesystem (production mount view via `wrap_scoped`), shared by
                // both stores so a raise and its resume round-trip through the
                // same records.
                let capability_store_filesystem =
                    ironclaw_reborn_composition::wrap_scoped(Arc::new(InMemoryBackend::new()));
                let gate_record_store: Arc<dyn ironclaw_run_state::GateRecordStore> =
                    Arc::new(ironclaw_run_state::FilesystemGateRecordStore::new(
                        Arc::clone(&capability_store_filesystem),
                    ));
                turn_executor_gate_store = Some(Arc::clone(&gate_record_store));
                let replay_payload_store: Arc<dyn ironclaw_capabilities::ReplayPayloadStore> =
                    Arc::new(ironclaw_capabilities::FilesystemReplayPayloadStore::new(
                        capability_store_filesystem,
                    ));
                Arc::new(ProductLiveHostRuntimeCapabilityFactory {
                    services: host_runtime_services.expect("host runtime services"),
                    io: host_runtime_io.expect("host runtime capability io"),
                    staged_inputs: Arc::clone(&host_runtime_staged_inputs),
                    invocations: Arc::clone(&capability_invocations),
                    results: Arc::clone(&capability_results),
                    capability_id: harness_capability_id(&capability.capability_id),
                    input: capability.input,
                    user_id: binding
                        .subject_user_id
                        .clone()
                        .expect("harness subject user id"),
                    cancellation_factory: cancellation_factory.clone(),
                    model_provider: config.model_provider.clone(),
                    model_id: config.model_id.clone(),
                    gate_record_store,
                    replay_payload_store,
                })
            } else if let Some(capability) = config.capability {
                Arc::new(RecordingCapabilityFactory {
                    capability,
                    invocations: Arc::clone(&capability_invocations),
                })
            } else {
                Arc::new(EmptyCapabilityFactory)
            };
        let model_route_resolver = Arc::new(
            StaticModelRouteResolver::new(ModelRoutePolicy::new(
                ModelSelectionMode::DeveloperAnyConfigured,
            ))
            .with_route(
                ModelSlot::Default,
                ModelRoute::new(config.model_provider, config.model_id)
                    .expect("valid harness model route"),
            ),
        );
        let capability_result_writer: Arc<dyn LoopCapabilityResultWriter> =
            Arc::new(ProductLiveCapabilityIo::default());
        let turn_state_for_runtime: Arc<dyn RuntimeTurnStateStore> = turn_store.clone();
        let await_edge_mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/turns").unwrap(),
            VirtualPath::new("/turns").unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .unwrap();
        let await_edge_store = Arc::new(FilesystemAwaitEdgeStore::new(Arc::new(
            ScopedFilesystem::with_fixed_view(Arc::new(InMemoryBackend::new()), await_edge_mounts),
        )));
        let await_edge_goal_store = Arc::new(in_memory_backed_subagent_goal_store());
        let await_edge_resolver = Arc::new(AwaitEdgeResolver::new_unbound(
            Arc::clone(&await_edge_store),
            await_edge_goal_store.clone() as Arc<dyn ironclaw_loop_host::SubagentSpawnGoalStore>,
            turn_store.clone() as Arc<dyn ironclaw_turns::TurnSpawnTreeStateStore>,
            capability_result_writer.clone(),
            Arc::new(thread_service.clone()),
        ));
        let await_edge_driver = Arc::new(ScopeRecoveryDriver::new(
            Arc::clone(&await_edge_resolver),
            Arc::clone(&await_edge_store),
        ));
        let composition = build_product_live_planned_runtime(DefaultPlannedRuntimeParts {
            attachment_read_port: None,
            gate_record_store: turn_executor_gate_store,
            turn_state: turn_state_for_runtime,
            thread_service: Arc::new(thread_service.clone()),
            thread_scope: thread_scope.clone(),
            model_gateway,
            checkpoint_state_store: in_memory_checkpoint_state_store(),
            loop_checkpoint_store: checkpoint_store.clone(),
            milestone_sink: Arc::new(InMemoryLoopHostMilestoneSink::default()),
            capability_factory,
            capability_surface_resolver: Arc::new(AllowAllCapabilitySurfaceResolver),
            capability_result_writer,
            subagent_goal_store: await_edge_goal_store,
            subagent_await_edge_writer: await_edge_driver
                as Arc<dyn ironclaw_loop_host::AwaitEdgeWriter>,
            subagent_await_edge_settler: await_edge_resolver
                as Arc<dyn ironclaw_loop_host::AwaitEdgeSettler>,
            subagent_await_edge_evidence: Arc::clone(&await_edge_store)
                as Arc<dyn ironclaw_runner::loop_exit_applier::AwaitDependentRunEvidenceStore>,
            subagent_definition_resolver: Arc::new(
                ironclaw_runner::subagent::flavors::StaticSubagentDefinitionResolver,
            ),
            subagent_spawn_input_codec: Arc::new(JsonSpawnSubagentInputCodec::new(Arc::new(
                ProductLiveCapabilityIo::default(),
            ))),
            subagent_spawn_limits: ironclaw_loop_host::SubagentSpawnLimits::default(),
            loop_exit_evidence: Arc::new(
                ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
                    Arc::new(thread_service.clone()),
                    Arc::clone(&turn_store) as Arc<dyn TurnStateStore>,
                    checkpoint_store,
                    await_edge_store
                        as Arc<
                            dyn ironclaw_runner::loop_exit_applier::AwaitDependentRunEvidenceStore,
                        >,
                    thread_scope.clone(),
                )
                .with_cancellation_factory(cancellation_factory.clone()),
            ),
            config: DefaultPlannedRuntimeConfig::default(),
            model_route_resolver: Some(model_route_resolver),
            cancellation_factory: Some(cancellation_factory.clone()),
            skill_context_source: None,
            input_queue: Some(Arc::new(EmptyInputQueue)),
            identity_context_source: Arc::new(EmptyIdentityContextSource),
            user_profile_source: Arc::new(EmptyUserProfileSource),
            model_policy_guard: Some(Arc::new(NoOpPolicyGuard)),
            model_budget_accountant: Some(Arc::new(NoOpBudgetAccountant)),
            safety_context: Some(test_safety_context()),
            hook_dispatcher_builder_factory: None,
            communication_context_provider: None,
            hook_security_audit_sink: None,
            turn_event_sink: None,
            scheduler_wake_wiring: None,
        })
        .expect("product-live planned AgentLoop harness should build");

        // The scheduler is started automatically inside build_product_live_planned_runtime.

        Self {
            binding_service,
            binding,
            thread_scope,
            thread_service,
            turn_store,
            cancellation_factory,
            composition,
            model_requests,
            capability_invocations,
            capability_results,
            model_release,
            _host_runtime_root: host_runtime_root,
        }
    }

    pub fn model_requests(&self) -> Vec<HostManagedModelRequest> {
        self.model_requests
            .lock()
            .expect("harness model requests lock poisoned")
            .clone()
    }

    pub fn capability_invocations(&self) -> Vec<LoopRequest> {
        self.capability_invocations
            .lock()
            .expect("harness capability invocation lock poisoned")
            .clone()
    }

    pub fn capability_results(&self) -> Vec<serde_json::Value> {
        self.capability_results
            .lock()
            .expect("harness capability results lock poisoned")
            .clone()
    }

    pub async fn wait_for_model_request_count(&self, expected: usize) {
        timeout(Duration::from_secs(3), async {
            loop {
                if self
                    .model_requests
                    .lock()
                    .expect("harness model requests lock poisoned")
                    .len()
                    >= expected
                {
                    return;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("harness model gateway should receive request count");
    }

    pub fn release_model(&self) {
        if let Some(release) = &self.model_release {
            release.cancel();
        }
    }

    pub fn user_message(&self, event_suffix: &str, text: &str) -> ProductInboundEnvelope {
        let envelope = user_message_envelope(event_suffix, text);
        self.binding_service
            .program_binding(envelope.source_binding_key(), self.binding.clone());
        envelope
    }

    pub async fn accept_user_message(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Result<InboundTurnOutcome, ironclaw_product::ProductWorkflowError> {
        let service = DefaultInboundTurnService::new(
            self.binding_service.clone(),
            self.thread_service.clone(),
            Arc::clone(&self.composition.coordinator),
        );
        service.accept_user_message(envelope).await
    }

    pub async fn wait_for_terminal(&self, run_id: TurnRunId) -> TurnRunState {
        let scope = self.turn_scope();
        timeout(Duration::from_secs(3), async {
            loop {
                let state = self
                    .turn_store
                    .get_run_state(GetRunStateRequest {
                        scope: scope.clone(),
                        run_id,
                    })
                    .await
                    .expect("harness run state");
                if state.status.is_terminal() {
                    return state;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("harness run should reach a terminal state")
    }

    pub async fn cancel_run(&self, run_id: TurnRunId) -> TurnStatus {
        self.composition
            .coordinator
            .cancel_run(CancelRunRequest {
                scope: self.turn_scope(),
                actor: TurnActor::new(self.binding.actor_user_id.clone()),
                run_id,
                reason: SanitizedCancelReason::UserRequested,
                idempotency_key: IdempotencyKey::new(format!("idem-harness-cancel-{run_id}"))
                    .expect("valid harness cancellation idempotency key"),
            })
            .await
            .expect("harness cancel run")
            .status
    }

    pub async fn wait_for_cancellation_observed(&self, run_id: TurnRunId) {
        timeout(Duration::from_secs(3), async {
            loop {
                if self
                    .cancellation_factory
                    .product_cancellation_observed(run_id)
                {
                    return;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("harness cancellation factory should observe run cancellation");
    }

    pub async fn thread_history(&self) -> Vec<ThreadMessageRecord> {
        self.thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: self.thread_scope.clone(),
                thread_id: self.binding.thread_id.clone(),
            })
            .await
            .expect("harness thread history")
            .messages
    }

    pub async fn shutdown(self) {
        self.composition.scheduler_handle.shutdown().await;
    }

    fn turn_scope(&self) -> TurnScope {
        TurnScope::new_with_owner(
            self.binding.tenant_id.clone(),
            self.binding.agent_id.clone(),
            self.binding.project_id.clone(),
            self.binding.thread_id.clone(),
            self.binding.subject_user_id.clone(),
        )
    }
}

#[derive(Debug)]
struct RecordingModelGateway {
    reply: String,
    requests: Arc<Mutex<Vec<HostManagedModelRequest>>>,
    responses: Mutex<VecDeque<HostManagedModelResponse>>,
    release: Option<CancellationToken>,
    host_runtime_tool_call: Option<ScriptedHostRuntimeToolCall>,
}

#[async_trait]
impl HostManagedModelGateway for RecordingModelGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        {
            let mut requests = self
                .requests
                .lock()
                .expect("recording model gateway requests lock poisoned");
            requests.push(request.clone());
        }
        if let Some(release) = &self.release {
            release.cancelled().await;
        }
        if let Some(tool_call) = &self.host_runtime_tool_call
            && let Some(response) = tool_call.response_for_request(&request).await?
        {
            return Ok(response);
        }
        if let Some(response) = self
            .responses
            .lock()
            .expect("recording model gateway responses lock poisoned")
            .pop_front()
        {
            return Ok(response);
        }
        Ok(HostManagedModelResponse::assistant_reply(
            self.reply.clone(),
        ))
    }
}

#[derive(Debug, Clone)]
struct ScriptedHostRuntimeToolCall {
    capability_id: CapabilityId,
    staged_inputs: Arc<Mutex<HashMap<TurnRunId, CapabilityInputRef>>>,
    issued_runs: Arc<Mutex<HashSet<TurnRunId>>>,
}

impl ScriptedHostRuntimeToolCall {
    async fn response_for_request(
        &self,
        request: &HostManagedModelRequest,
    ) -> Result<Option<HostManagedModelResponse>, HostManagedModelError> {
        {
            let mut issued_runs = self
                .issued_runs
                .lock()
                .expect("host-runtime issued runs lock poisoned");
            if !issued_runs.insert(request.run_id) {
                return Ok(None);
            }
        }
        let input_ref = self.wait_for_input_ref(request.run_id).await?;
        let Some(surface_version) = request.surface_version.clone() else {
            return Err(HostManagedModelError::safe(
                HostManagedModelErrorKind::InvalidRequest,
                "capability tool call requires a visible surface version",
            ));
        };
        Ok(Some(HostManagedModelResponse {
            safe_text_deltas: Vec::new(),
            safe_reasoning_deltas: Vec::new(),
            usage: None,
            output: ParentLoopOutput::CapabilityCalls(vec![CapabilityCallCandidate {
                activity_id: ironclaw_turns::CapabilityActivityId::new(),
                surface_version,
                capability_id: self.capability_id.clone(),
                input_ref,
                effective_capability_ids: vec![self.capability_id.clone()],
                provider_replay: None,
            }]),
        }))
    }

    async fn wait_for_input_ref(
        &self,
        run_id: TurnRunId,
    ) -> Result<CapabilityInputRef, HostManagedModelError> {
        timeout(Duration::from_secs(3), async {
            loop {
                if let Some(input_ref) = self
                    .staged_inputs
                    .lock()
                    .expect("host-runtime staged input lock poisoned")
                    .get(&run_id)
                    .cloned()
                {
                    return input_ref;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .map_err(|_| {
            HostManagedModelError::safe(
                HostManagedModelErrorKind::Unavailable,
                "timed out waiting for host-runtime staged tool input",
            )
        })
    }
}

struct ProductLiveHostRuntimeCapabilityFactory {
    services: Arc<RebornRuntime>,
    io: Arc<ProductLiveCapabilityIo>,
    staged_inputs: Arc<Mutex<HashMap<TurnRunId, CapabilityInputRef>>>,
    invocations: Arc<Mutex<Vec<LoopRequest>>>,
    results: Arc<Mutex<Vec<serde_json::Value>>>,
    capability_id: CapabilityId,
    input: serde_json::Value,
    user_id: UserId,
    cancellation_factory: Arc<ReadyRunCancellationFactory>,
    model_provider: String,
    model_id: String,
    // Durable gate-record + replay-payload stores wired into the ProductLive
    // capability port, so a raise and its later resume round-trip through the
    // SAME store (both built over one in-memory filesystem below). Modeling the
    // production wiring the local-dev path already has (#6287).
    gate_record_store: Arc<dyn ironclaw_run_state::GateRecordStore>,
    replay_payload_store: Arc<dyn ironclaw_capabilities::ReplayPayloadStore>,
}

#[async_trait]
impl LoopCapabilityPortFactory for ProductLiveHostRuntimeCapabilityFactory {
    async fn create_capability_port(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        let input_ref = self
            .io
            .stage_input(run_context, self.input.clone())
            .map_err(|error| {
                AgentLoopHostError::new(error.kind, format!("failed to stage tool input: {error}"))
            })?;
        self.staged_inputs
            .lock()
            .expect("host-runtime staged input lock poisoned")
            .insert(run_context.run_id, input_ref);
        let visible_capability_request = ProductLiveVisibleCapabilityRequestConfig::new(
            self.user_id.clone(),
            RuntimeKind::FirstParty,
            TrustClass::FirstParty,
            SurfaceKind::new("agent_loop").expect("valid surface kind"),
            CapabilitySurfacePolicy::allow_all(),
        )
        .with_grants(dispatch_grants_for_user(
            self.user_id.clone(),
            [&self.capability_id],
        ))
        .with_provider_trust(
            ExtensionId::new("builtin").expect("valid builtin provider id"),
            EffectiveTrustClass::user_trusted(),
        );
        let adapters = ProductLivePlannedRuntimeAdapters::from_host_runtime(
            self.services
                .host_runtime_for_test()
                .expect("host runtime harness services"),
            ProductLivePlannedRuntimeAdapterConfig {
                capability_authority_resolver: Arc::new(StaticProductLiveAuthorityResolver {
                    config: visible_capability_request,
                }),
                capability_input_resolver: self.io.clone(),
                capability_result_writer: self.io.clone(),
                capability_allow_set: capability_allowlist([self.capability_id.clone()]),
                model_routes: ProductLiveModelRouteSettings::new(
                    self.model_provider.clone(),
                    self.model_id.clone(),
                )
                .map_err(adapter_error)?,
                cancellation_factory: self.cancellation_factory.clone(),
                input_queue: Arc::new(EmptyInputQueue),
                identity_context_source: Arc::new(EmptyIdentityContextSource),
                model_policy_guard: Arc::new(NoOpPolicyGuard),
                model_budget_accountant: Arc::new(NoOpBudgetAccountant),
                safety_context: test_safety_context(),
                milestone_sink: Arc::new(InMemoryLoopHostMilestoneSink::default()),
                gate_record_store: Arc::clone(&self.gate_record_store),
                replay_payload_store: Arc::clone(&self.replay_payload_store),
            },
        )
        .map_err(adapter_error)?;
        adapters
            .capability_factory
            .create_capability_port(run_context)
            .await
            .map(|inner| {
                Arc::new(RecordingDelegatingCapabilityPort {
                    inner,
                    run_context: run_context.clone(),
                    io: Arc::clone(&self.io),
                    invocations: Arc::clone(&self.invocations),
                    results: Arc::clone(&self.results),
                }) as Arc<dyn LoopCapabilityPort>
            })
    }
}

struct RecordingDelegatingCapabilityPort {
    inner: Arc<dyn LoopCapabilityPort>,
    run_context: LoopRunContext,
    io: Arc<ProductLiveCapabilityIo>,
    invocations: Arc<Mutex<Vec<LoopRequest>>>,
    results: Arc<Mutex<Vec<serde_json::Value>>>,
}

#[async_trait]
impl LoopCapabilityPort for RecordingDelegatingCapabilityPort {
    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        self.inner.visible_capabilities(request).await
    }

    async fn invoke_capability(
        &self,
        request: LoopRequest,
    ) -> Result<Resolution, AgentLoopHostError> {
        self.invocations
            .lock()
            .expect("harness capability invocation lock poisoned")
            .push(request.clone());
        let resolution = self.inner.invoke_capability(request).await?;
        self.record_completed_result(&resolution)?;
        Ok(resolution)
    }

    async fn invoke_capability_batch(
        &self,
        request: LoopRequestBatch,
    ) -> Result<ResolutionBatch, AgentLoopHostError> {
        self.invocations
            .lock()
            .expect("harness capability invocation lock poisoned")
            .extend(request.invocations.iter().cloned());
        let batch = self.inner.invoke_capability_batch(request).await?;
        for item in &batch.resolutions {
            self.record_completed_result(item)?;
        }
        Ok(batch)
    }
}

impl RecordingDelegatingCapabilityPort {
    fn record_completed_result(&self, resolution: &Resolution) -> Result<(), AgentLoopHostError> {
        // Only a successful `Done` staged output to record (the flip's acceptance
        // table maps the old `Completed` variant to `Done` + `ToolVerdict::Success`).
        // The host mints an opaque `refs.result` uuid; the originating loop result
        // ref the staged value is keyed by is preserved on `refs.origin`.
        let Resolution::Done(outcome) = resolution else {
            return Ok(());
        };
        if !outcome.verdict.is_success() {
            return Ok(());
        }
        let Some(origin) = outcome.refs.origin.as_ref() else {
            return Ok(());
        };
        let result_ref = LoopResultRef::new(origin.as_str()).map_err(|error| {
            AgentLoopHostError::new(
                ironclaw_turns::run_profile::AgentLoopHostErrorKind::InvalidInvocation,
                format!("invalid preserved loop result ref: {error}"),
            )
        })?;
        let value = self.io.result_for_ref(&self.run_context, &result_ref)?;
        self.results
            .lock()
            .expect("harness capability results lock poisoned")
            .push(value);
        Ok(())
    }
}

struct StaticProductLiveAuthorityResolver {
    config: ProductLiveVisibleCapabilityRequestConfig,
}

#[async_trait]
impl ProductLiveCapabilityAuthorityResolver for StaticProductLiveAuthorityResolver {
    async fn resolve_capability_authority(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<ProductLiveVisibleCapabilityRequestConfig, ProductLivePlannedRuntimeAdapterError>
    {
        Ok(self.config.clone())
    }
}

struct RecordingCapabilityFactory {
    capability: HarnessCapabilityConfig,
    invocations: Arc<Mutex<Vec<LoopRequest>>>,
}

#[async_trait]
impl LoopCapabilityPortFactory for RecordingCapabilityFactory {
    async fn create_capability_port(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        Ok(Arc::new(RecordingCapabilityPort {
            capability: self.capability.clone(),
            invocations: Arc::clone(&self.invocations),
        }))
    }
}

struct RecordingCapabilityPort {
    capability: HarnessCapabilityConfig,
    invocations: Arc<Mutex<Vec<LoopRequest>>>,
}

#[async_trait]
impl LoopCapabilityPort for RecordingCapabilityPort {
    async fn visible_capabilities(
        &self,
        _request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        Ok(VisibleCapabilitySurface {
            callable_capability_ids: None,
            version: harness_surface_version(),
            descriptors: vec![CapabilityDescriptorView {
                capability_id: harness_capability_id(&self.capability.capability_id),
                provider: Some(ExtensionId::new("harness.provider").expect("valid provider id")),
                runtime: RuntimeKind::FirstParty,
                safe_name: self.capability.capability_id.clone(),
                safe_description: "harness capability".to_string(),
                parameters_schema: serde_json::json!({ "type": "object" }),
                concurrency_hint: ConcurrencyHint::Exclusive,
            }],
        })
    }

    async fn invoke_capability(
        &self,
        request: LoopRequest,
    ) -> Result<Resolution, AgentLoopHostError> {
        self.invocations
            .lock()
            .expect("harness capability invocation lock poisoned")
            .push(request);
        let outcome = resolution::completed(
            LoopResultRef::new(self.capability.result_ref.clone())
                .expect("valid harness result ref"),
            self.capability.safe_summary.clone(),
            ironclaw_turns::run_profile::CapabilityProgress::MadeProgress,
            self.capability.terminate_hint,
            0,
            None,
            None,
        );
        Ok(outcome)
    }

    async fn invoke_capability_batch(
        &self,
        request: LoopRequestBatch,
    ) -> Result<ResolutionBatch, AgentLoopHostError> {
        let mut resolutions = Vec::new();
        let mut stopped_on_suspension = false;
        for invocation in request.invocations {
            let resolution = self.invoke_capability(invocation).await?;
            // `parks()` is the batch-stop predicate (true for gates and
            // suspensions); this producer only ever completes, so it never parks.
            stopped_on_suspension |= request.stop_on_first_suspension && resolution.parks();
            resolutions.push(resolution);
            if stopped_on_suspension {
                break;
            }
        }
        Ok(ResolutionBatch {
            resolutions,
            stopped_on_suspension,
        })
    }
}

struct EmptyCapabilityFactory;

#[async_trait]
impl LoopCapabilityPortFactory for EmptyCapabilityFactory {
    async fn create_capability_port(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        Ok(Arc::new(EmptyLoopCapabilityPort))
    }
}

struct AllowAllCapabilitySurfaceResolver;

#[async_trait]
impl CapabilitySurfaceProfileResolver for AllowAllCapabilitySurfaceResolver {
    async fn resolve(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<CapabilityAllowSet, CapabilityResolveError> {
        Ok(CapabilityAllowSet::All)
    }
}

struct EmptyInputQueue;

#[async_trait]
impl HostInputQueue for EmptyInputQueue {
    async fn next_after(
        &self,
        _run_id: TurnRunId,
        after: LoopInputCursorToken,
        _limit: usize,
    ) -> Result<HostInputBatch, HostInputQueueError> {
        Ok(HostInputBatch {
            inputs: Vec::new(),
            next_cursor: after,
        })
    }

    async fn ack_consumed(
        &self,
        _run_id: TurnRunId,
        _tokens: Vec<LoopInputAckToken>,
    ) -> Result<(), HostInputQueueError> {
        Ok(())
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

#[derive(Default)]
struct ReadyRunCancellationFactory {
    handles: Arc<Mutex<HashMap<TurnRunId, RunCancellationHandle>>>,
}

impl ReadyRunCancellationFactory {
    fn handle_for(&self, run_id: TurnRunId) -> Option<RunCancellationHandle> {
        self.handles
            .lock()
            .expect("ready cancellation lock poisoned")
            .get(&run_id)
            .cloned()
    }

    fn product_cancellation_observed(&self, run_id: TurnRunId) -> bool {
        self.handle_for(run_id)
            .map(|handle| handle.is_requested())
            .unwrap_or(false)
    }
}

#[async_trait]
impl RunCancellationFactory for ReadyRunCancellationFactory {
    async fn handle_for_run(
        &self,
        _scope: &TurnScope,
        run_id: TurnRunId,
    ) -> Result<RunCancellationHandle, AgentLoopHostError> {
        let handle = RunCancellationHandle::default();
        self.handles
            .lock()
            .expect("ready cancellation lock poisoned")
            .insert(run_id, handle.clone());
        Ok(handle)
    }

    fn notify_run_wake(&self, wake: &TurnRunWake) {
        if wake.status != TurnStatus::CancelRequested {
            return;
        }
        if let Some(handle) = self.handle_for(wake.run_id) {
            handle.request(LoopCancelReasonKind::UserRequested);
        }
    }

    fn product_live_cancellation_probe(&self) -> Option<Box<dyn ProductLiveCancellationProbe>> {
        let run_id = TurnRunId::new();
        let handle = RunCancellationHandle::default();
        self.handles
            .lock()
            .expect("ready cancellation lock poisoned")
            .insert(run_id, handle);
        Some(Box::new(ReadyRunCancellationProbe {
            handles: Arc::clone(&self.handles),
            run_id,
        }))
    }

    fn is_product_cancellation_observed(
        &self,
        run_id: TurnRunId,
    ) -> Result<bool, AgentLoopHostError> {
        Ok(self.product_cancellation_observed(run_id))
    }
}

struct ReadyRunCancellationProbe {
    handles: Arc<Mutex<HashMap<TurnRunId, RunCancellationHandle>>>,
    run_id: TurnRunId,
}

impl ReadyRunCancellationProbe {
    fn handle(&self) -> RunCancellationHandle {
        self.handles
            .lock()
            .expect("ready cancellation lock poisoned")
            .get(&self.run_id)
            .cloned()
            .expect("probe handle retained for readiness check")
    }
}

impl Drop for ReadyRunCancellationProbe {
    fn drop(&mut self) {
        self.handles
            .lock()
            .expect("ready cancellation lock poisoned")
            .remove(&self.run_id);
    }
}

impl ProductLiveCancellationProbe for ReadyRunCancellationProbe {
    fn request_cancellation(
        &self,
        reason_kind: LoopCancelReasonKind,
    ) -> Result<(), AgentLoopHostError> {
        self.handle().request(reason_kind);
        Ok(())
    }

    fn is_cancellation_observed(&self) -> Result<bool, AgentLoopHostError> {
        Ok(self.handle().is_requested())
    }
}

fn user_message_envelope(event_suffix: &str, text: &str) -> ProductInboundEnvelope {
    let installation_id = "install_harness";
    let evidence = ProtocolAuthEvidence::test_verified(
        AuthRequirement::SharedSecretHeader {
            header_name: "X-Secret".into(),
        },
        installation_id,
    );
    let context = TrustedInboundContext::from_verified_evidence(
        ProductAdapterId::new("test_adapter").expect("valid adapter id"),
        AdapterInstallationId::new(installation_id).expect("valid installation id"),
        Utc::now(),
        &evidence,
    )
    .expect("verified inbound context");
    let parsed = ParsedProductInbound::new(
        ExternalEventId::new(format!("evt:{event_suffix}")).expect("valid event id"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("valid actor ref"),
        ExternalConversationRef::new(None, "conv1", None, None).expect("valid conversation ref"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new(text, vec![], ProductTriggerReason::DirectChat)
                .expect("valid user message"),
        ),
    )
    .expect("parsed inbound");

    ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("trusted envelope")
}

fn test_safety_context() -> InstructionSafetyContext {
    InstructionSafetyContext::new("policy:test", "test safety context")
        .expect("test safety context")
}

fn harness_surface_version() -> CapabilitySurfaceVersion {
    CapabilitySurfaceVersion::new("surface:harness-v1").expect("valid harness surface version")
}

fn harness_capability_id(capability_id: impl Into<String>) -> CapabilityId {
    CapabilityId::new(capability_id.into()).expect("valid harness capability id")
}

fn dispatch_grants_for_user<const N: usize>(
    user_id: UserId,
    capabilities: [&CapabilityId; N],
) -> CapabilitySet {
    CapabilitySet {
        grants: capabilities
            .into_iter()
            .map(|capability| CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: capability.clone(),
                grantee: Principal::User(user_id.clone()),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::DispatchCapability],
                    mounts: ironclaw_host_api::MountView::default(),
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

fn adapter_error(error: impl Display) -> AgentLoopHostError {
    AgentLoopHostError::new(
        ironclaw_turns::run_profile::AgentLoopHostErrorKind::Internal,
        error.to_string(),
    )
}
