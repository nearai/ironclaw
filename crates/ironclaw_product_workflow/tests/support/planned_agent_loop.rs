use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{AgentId, TenantId, ThreadId, UserId};
use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileResolver,
    EmptyLoopCapabilityPort, HostIdentityContextBuildError, HostIdentityContextCandidate,
    HostIdentityContextSource, HostInputBatch, HostInputQueue, HostInputQueueError,
    HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse, ProductLiveCancellationProbe, RunCancellationFactory,
    RunCancellationHandle,
};
use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, ExternalActorRef, ExternalConversationRef,
    ExternalEventId, ParsedProductInbound, ProductAdapterId, ProductInboundEnvelope,
    ProductInboundPayload, ProductTriggerReason, ProtocolAuthEvidence, TrustedInboundContext,
    UserMessagePayload,
};
use ironclaw_product_workflow::{
    DefaultInboundTurnService, FakeConversationBindingService, InboundTurnOutcome,
    InboundTurnService, ResolvedBinding,
};
use ironclaw_reborn::loop_driver_host::LoopCapabilityPortFactory;
use ironclaw_reborn::loop_exit_applier::ThreadCheckpointLoopExitEvidencePort;
use ironclaw_reborn::model_routes::{
    ModelRoute, ModelRoutePolicy, ModelSelectionMode, ModelSlot, StaticModelRouteResolver,
};
use ironclaw_reborn::runtime::{
    DefaultPlannedRuntimeConfig, DefaultPlannedRuntimeParts, RebornRuntimeLoopComposition,
    build_product_live_planned_runtime,
};
use ironclaw_threads::{
    InMemorySessionThreadService, SessionThreadService, ThreadHistoryRequest, ThreadMessageRecord,
    ThreadScope,
};
use ironclaw_turns::{
    CancelRunRequest, GetRunStateRequest, IdempotencyKey, InMemoryCheckpointStateStore,
    InMemoryLoopCheckpointStore, InMemoryTurnStateStore, SanitizedCancelReason, TurnActor,
    TurnCoordinator, TurnRunId, TurnRunState, TurnRunWake, TurnScope, TurnStateStore, TurnStatus,
    run_profile::{
        AgentLoopHostError, InMemoryLoopHostMilestoneSink, InstructionSafetyContext,
        LoopCancelReasonKind, LoopCapabilityPort, LoopInputAckToken, LoopInputCursorToken,
        LoopRunContext, NoOpBudgetAccountant, NoOpPolicyGuard, PromptMode,
    },
};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;

pub struct ProductLiveAgentLoopHarness {
    binding_service: FakeConversationBindingService,
    binding: ResolvedBinding,
    thread_scope: ThreadScope,
    thread_service: InMemorySessionThreadService,
    turn_store: Arc<InMemoryTurnStateStore>,
    cancellation_factory: Arc<ReadyRunCancellationFactory>,
    composition: RebornRuntimeLoopComposition<
        InMemoryTurnStateStore,
        InMemorySessionThreadService,
        RecordingModelGateway,
    >,
    model_requests: Arc<Mutex<Vec<HostManagedModelRequest>>>,
    model_release: Option<CancellationToken>,
    worker_cancel: CancellationToken,
    worker_handle: JoinHandle<()>,
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
        }
    }
}

impl ProductLiveAgentLoopHarness {
    pub async fn new(config: ProductLiveAgentLoopHarnessConfig) -> Self {
        let binding_service = FakeConversationBindingService::new();
        let binding = ResolvedBinding {
            tenant_id: TenantId::new(config.tenant_id).expect("valid harness tenant id"),
            user_id: UserId::new(config.user_id).expect("valid harness user id"),
            thread_id: ThreadId::new(config.thread_id).expect("valid harness thread id"),
            agent_id: Some(AgentId::new(config.agent_id).expect("valid harness agent id")),
            project_id: None,
        };
        let thread_scope = ThreadScope {
            tenant_id: binding.tenant_id.clone(),
            agent_id: binding.agent_id.clone().expect("harness agent id"),
            project_id: binding.project_id.clone(),
            owner_user_id: Some(binding.user_id.clone()),
            mission_id: None,
        };
        let thread_service = InMemorySessionThreadService::default();
        let turn_store = Arc::new(InMemoryTurnStateStore::default());
        let checkpoint_store = Arc::new(InMemoryLoopCheckpointStore::default());
        let model_requests = Arc::new(Mutex::new(Vec::new()));
        let model_release = config
            .pause_model_until_released
            .then(CancellationToken::new);
        let model_gateway = Arc::new(RecordingModelGateway {
            reply: config.assistant_reply,
            requests: Arc::clone(&model_requests),
            release: model_release.clone(),
        });
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
        let cancellation_factory = Arc::new(ReadyRunCancellationFactory::default());
        let composition = build_product_live_planned_runtime(DefaultPlannedRuntimeParts {
            turn_state: Arc::clone(&turn_store),
            thread_service: Arc::new(thread_service.clone()),
            thread_scope: thread_scope.clone(),
            model_gateway,
            checkpoint_state_store: Arc::new(InMemoryCheckpointStateStore::default()),
            loop_checkpoint_store: checkpoint_store.clone(),
            milestone_sink: Arc::new(InMemoryLoopHostMilestoneSink::default()),
            capability_factory: Arc::new(EmptyCapabilityFactory),
            capability_surface_resolver: Arc::new(AllowAllCapabilitySurfaceResolver),
            loop_exit_evidence: Arc::new(
                ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
                    Arc::new(thread_service.clone()),
                    Arc::clone(&turn_store) as Arc<dyn TurnStateStore>,
                    checkpoint_store,
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
            model_policy_guard: Some(Arc::new(NoOpPolicyGuard)),
            model_budget_accountant: Some(Arc::new(NoOpBudgetAccountant)),
            safety_context: Some(test_safety_context()),
        })
        .expect("product-live planned AgentLoop harness should build");

        let worker_cancel = CancellationToken::new();
        let worker = Arc::clone(&composition.worker);
        let worker_cancel_clone = worker_cancel.clone();
        let worker_handle = tokio::spawn(async move { worker.run(worker_cancel_clone).await });

        Self {
            binding_service,
            binding,
            thread_scope,
            thread_service,
            turn_store,
            cancellation_factory,
            composition,
            model_requests,
            model_release,
            worker_cancel,
            worker_handle,
        }
    }

    pub fn model_requests(&self) -> Vec<HostManagedModelRequest> {
        self.model_requests
            .lock()
            .expect("harness model requests lock poisoned")
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
    ) -> Result<InboundTurnOutcome, ironclaw_product_workflow::ProductWorkflowError> {
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
                actor: TurnActor::new(self.binding.user_id.clone()),
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
        self.worker_cancel.cancel();
        self.worker_handle
            .await
            .expect("harness worker should stop cleanly");
    }

    fn turn_scope(&self) -> TurnScope {
        TurnScope::new(
            self.binding.tenant_id.clone(),
            self.binding.agent_id.clone(),
            self.binding.project_id.clone(),
            self.binding.thread_id.clone(),
        )
    }
}

#[derive(Debug)]
struct RecordingModelGateway {
    reply: String,
    requests: Arc<Mutex<Vec<HostManagedModelRequest>>>,
    release: Option<CancellationToken>,
}

#[async_trait]
impl HostManagedModelGateway for RecordingModelGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.requests
            .lock()
            .expect("recording model gateway requests lock poisoned")
            .push(request);
        if let Some(release) = &self.release {
            release.cancelled().await;
        }
        Ok(HostManagedModelResponse::assistant_reply(
            self.reply.clone(),
        ))
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
    fn product_cancellation_observed(&self, run_id: TurnRunId) -> bool {
        self.handles
            .lock()
            .expect("ready cancellation lock poisoned")
            .get(&run_id)
            .map(RunCancellationHandle::is_requested)
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
        if let Some(handle) = self
            .handles
            .lock()
            .expect("ready cancellation lock poisoned")
            .get(&wake.run_id)
            .cloned()
        {
            handle.request(LoopCancelReasonKind::UserRequested);
        }
    }

    fn product_live_cancellation_probe(&self) -> Option<Box<dyn ProductLiveCancellationProbe>> {
        Some(Box::new(ReadyRunCancellationProbe {
            handle: RunCancellationHandle::default(),
        }))
    }

    fn is_product_cancellation_observed(
        &self,
        run_id: TurnRunId,
    ) -> Result<bool, AgentLoopHostError> {
        Ok(self
            .handles
            .lock()
            .expect("ready cancellation lock poisoned")
            .get(&run_id)
            .map(RunCancellationHandle::is_requested)
            .unwrap_or(false))
    }
}

struct ReadyRunCancellationProbe {
    handle: RunCancellationHandle,
}

impl ProductLiveCancellationProbe for ReadyRunCancellationProbe {
    fn request_cancellation(
        &self,
        reason_kind: LoopCancelReasonKind,
    ) -> Result<(), AgentLoopHostError> {
        self.handle.request(reason_kind);
        Ok(())
    }

    fn is_cancellation_observed(&self) -> Result<bool, AgentLoopHostError> {
        Ok(self.handle.is_requested())
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
